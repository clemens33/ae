//! Console input, pure: terminal reads to keys, keys to one composed line, a
//! line to a command, and the step machine that says what each act does. The
//! reads, the tmux readings and the writes live in `console::term`.

use std::time::Instant;

use super::submit::{Draft, Outcome};
use crate::board::terminal_text;

/// The most bytes one composed line holds: the draft file's own cap.
pub const CAP: usize = 65_536;
/// An escape sequence still open past this many bytes is dropped whole.
pub const PENDING_MAX: usize = 16;
/// The most rows the composer takes, the elision marker rows included.
pub const ROWS_MAX: usize = 10;
/// Bracketed paste on: the terminal marks every paste.
pub const PASTE_ON: &str = "\x1b[?2004h";
/// Bracketed paste off.
pub const PASTE_OFF: &str = "\x1b[?2004l";
/// Opens every composer draw: autowrap off.
pub const DRAW_OPEN: &str = "\x1b[?7l";
/// Closes every composer draw, in the same write: autowrap back on.
pub const DRAW_CLOSE: &str = "\x1b[?7h";
/// Written on every exit: autowrap on, bracketed paste off.
pub const RESTORE: &str = "\x1b[?7h\x1b[?2004l";

const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

/// One key, as the composer takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// Literal bytes: a typed run, or anything inside a paste.
    Text(Vec<u8>),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    ClearLine,
    Enter,
}

/// Terminal reads cut into keys. Each key carries the stamp of the read that
/// held its first byte — for text inside a paste, the read that held the
/// paste's opening `ESC` — so a key begun before some moment is known as such
/// however the reads split it. A paste is literal until its own close or the
/// end of input: nothing inside one is ever Enter, ^U or an erase.
#[derive(Debug, Default)]
pub struct Keys {
    /// An escape sequence begun and not yet complete.
    pending: Vec<u8>,
    /// The stamp of the read that began `pending` outside a paste.
    begun: Option<Instant>,
    /// The open paste's origin.
    paste: Option<Instant>,
}

impl Keys {
    /// The keys `chunk` completes, read at `stamp`.
    pub fn feed(&mut self, chunk: &[u8], stamp: Instant) -> Vec<(Key, Instant)> {
        let mut keys = Vec::new();
        for &byte in chunk {
            if let Some(origin) = self.paste {
                if self.pending.is_empty() && byte != 0x1b {
                    text(&mut keys, &[byte], origin);
                    continue;
                }
                self.pending.push(byte);
                if self.pending == PASTE_END {
                    self.pending.clear();
                    self.paste = None;
                } else if !PASTE_END.starts_with(&self.pending) {
                    let mut held = std::mem::take(&mut self.pending);
                    if held.len() > 1 && byte == 0x1b {
                        held.pop();
                        self.pending.push(byte);
                    }
                    text(&mut keys, &held, origin);
                }
            } else if !self.pending.is_empty() || byte == 0x1b {
                if self.pending.is_empty() {
                    self.begun = Some(stamp);
                }
                self.pending.push(byte);
                if self.pending == PASTE_START {
                    self.paste = self.begun.take();
                    self.pending.clear();
                } else if escape_done(&self.pending) || self.pending.len() > PENDING_MAX {
                    let began = self.begun.take();
                    let key = editing(&std::mem::take(&mut self.pending));
                    keys.extend(key.zip(began));
                }
            } else {
                let key = match byte {
                    0x7f | 0x08 => Key::Backspace,
                    0x01 => Key::Home,
                    0x05 => Key::End,
                    0x15 => Key::ClearLine,
                    b'\r' | b'\n' => Key::Enter,
                    _ => {
                        text(&mut keys, &[byte], stamp);
                        continue;
                    }
                };
                keys.push((key, stamp));
            }
        }
        keys
    }
}

/// Append `bytes` to the last key when it is text from the same origin.
fn text(keys: &mut Vec<(Key, Instant)>, bytes: &[u8], origin: Instant) {
    if let Some((Key::Text(run), at)) = keys.last_mut()
        && *at == origin
    {
        run.extend_from_slice(bytes);
        return;
    }
    keys.push((Key::Text(bytes.to_vec()), origin));
}

/// Whether escape sequence `seq` is complete: a CSI at its final byte, an
/// SS3 after its one byte, any other `ESC x` at once.
fn escape_done(seq: &[u8]) -> bool {
    match seq {
        [0x1b] | [0x1b, b'[' | b'O'] => false,
        [0x1b, b'[', .., last] => (0x40..=0x7e).contains(last),
        _ => true,
    }
}

/// The editing key a complete escape sequence spells, in either cursor-key
/// mode; any other sequence is no key.
fn editing(seq: &[u8]) -> Option<Key> {
    Some(match seq {
        b"\x1b[D" | b"\x1bOD" => Key::Left,
        b"\x1b[C" | b"\x1bOC" => Key::Right,
        b"\x1b[H" | b"\x1bOH" | b"\x1b[1~" | b"\x1b[7~" => Key::Home,
        b"\x1b[F" | b"\x1bOF" | b"\x1b[4~" | b"\x1b[8~" => Key::End,
        b"\x1b[3~" => Key::Delete,
        _ => return None,
    })
}

/// What Enter made of the composed bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entered {
    Line(Vec<u8>),
    Over,
    Empty,
}

/// One composed line: literal bytes up to [`CAP`], and the byte offset the
/// cursor is at, where text goes in and an erase takes out. Past the cap it is
/// over — later bytes are dropped and Enter refuses — until ^U clears it.
#[derive(Debug, Default)]
pub struct Composer {
    bytes: Vec<u8>,
    cursor: usize,
    over: bool,
}

impl Composer {
    /// Take one key; `Some` for Enter alone.
    pub fn key(&mut self, key: Key) -> Option<Entered> {
        if !matches!(key, Key::Text(_)) {
            self.snap();
        }
        match key {
            Key::Text(text) if !self.over => {
                if self.bytes.len() + text.len() > CAP {
                    self.over = true;
                } else {
                    self.bytes
                        .splice(self.cursor..self.cursor, text.iter().copied());
                    self.cursor += text.len();
                }
            }
            Key::Backspace if !self.over => {
                let from = self.unit(false);
                self.bytes.drain(from..self.cursor);
                self.cursor = from;
            }
            Key::Delete if !self.over => {
                self.bytes.drain(self.cursor..self.unit(true));
            }
            Key::Left if !self.over => self.cursor = self.unit(false),
            Key::Right if !self.over => self.cursor = self.unit(true),
            Key::Home if !self.over => self.cursor = 0,
            Key::End if !self.over => self.cursor = self.bytes.len(),
            Key::ClearLine => *self = Self::default(),
            Key::Enter if self.over => return Some(Entered::Over),
            Key::Enter if self.bytes.is_empty() => return Some(Entered::Empty),
            Key::Enter => {
                self.cursor = 0;
                return Some(Entered::Line(std::mem::take(&mut self.bytes)));
            }
            Key::Text(_)
            | Key::Backspace
            | Key::Delete
            | Key::Left
            | Key::Right
            | Key::Home
            | Key::End => {}
        }
        None
    }

    /// Where each unit of the draft ends, 0 first: a character, or a byte that
    /// is not UTF-8.
    fn ends(&self) -> Vec<usize> {
        let mut ends = vec![0];
        for chunk in self.bytes.utf8_chunks() {
            let valid = chunk.valid().chars().map(char::len_utf8);
            for len in valid.chain(chunk.invalid().iter().map(|_| 1)) {
                ends.push(ends[ends.len() - 1] + len);
            }
        }
        ends
    }

    /// The cursor as a key that is not text and the layout read it: on the end of
    /// the character it sits inside, since text goes in at any byte offset.
    fn snapped(&self) -> usize {
        let next = self.ends().into_iter().find(|&end| end >= self.cursor);
        next.unwrap_or(self.bytes.len())
    }

    fn snap(&mut self) {
        self.cursor = self.snapped();
    }

    /// The offset one unit from the cursor, the cursor itself at either end.
    fn unit(&self, forward: bool) -> usize {
        let ends = self.ends();
        let beyond = |end: &usize| {
            if forward {
                *end > self.cursor
            } else {
                *end < self.cursor
            }
        };
        let found = if forward {
            ends.into_iter().find(beyond)
        } else {
            ends.into_iter().rev().find(beyond)
        };
        found.unwrap_or(self.cursor)
    }

    /// `prompt`, then the whole draft on one line — never a byte a terminal
    /// would act on — or, past the cap, the notice saying so.
    #[must_use]
    pub fn line(&self, prompt: &str) -> String {
        if self.over {
            return format!("{prompt}{}", over_notice());
        }
        let draft = String::from_utf8_lossy(&self.bytes).replace(['\n', '\r', '\t'], " ");
        format!("{prompt}{}", terminal_text(&draft))
    }

    /// The draft laid out for a pane of `size`, `prompt` on its first row.
    #[must_use]
    pub fn view(&self, prompt: &str, size: Size) -> View {
        let width = size.width.max(1);
        let keep = width.div_ceil(2).min(3);
        let indent = prompt.chars().map(cells).sum::<usize>().min(width - keep);
        let anchor = clip(prompt, indent);
        let pad = " ".repeat(indent - anchor.chars().map(cells).sum::<usize>());
        let (first, rest) = (format!("{anchor}{pad}"), " ".repeat(indent));
        let area = width - indent;
        let (texts, row, before) = if self.over {
            let row = clip(&over_notice(), area);
            (vec![row.clone()], 0, row)
        } else {
            self.wrapped(area)
        };
        let cap = ROWS_MAX.min(size.height.saturating_sub(1).max(1));
        let win = window(texts.len(), row, cap);
        let mut shown = Vec::new();
        if win.above {
            shown.push(clip(&format!("… +{} lines above", win.start), area));
        }
        shown.extend_from_slice(&texts[win.start..win.end]);
        if win.below {
            let hidden = texts.len() - win.end;
            shown.push(clip(&format!("… +{hidden} lines below"), area));
        }
        let cursor_row = usize::from(win.above) + row - win.start;
        let lead = |at: usize| if at == 0 { &first } else { &rest };
        let rows = (shown.iter().enumerate())
            .map(|(at, text)| format!("{}{text}", lead(at)))
            .collect();
        View {
            rows,
            cursor_row,
            before: format!("{}{before}", lead(cursor_row)),
            anchor,
        }
    }

    /// The draft in rows of at most `area` cells, the row the cursor is on and
    /// that row up to it. A break is `\n` or a `\r` not followed by `\n`; a
    /// byte that is not UTF-8 is its own unit, drawn as U+FFFD.
    fn wrapped(&self, area: usize) -> (Vec<String>, usize, String) {
        let mut rows = Rows::new(area, self.snapped());
        let mut at = 0;
        for chunk in self.bytes.utf8_chunks() {
            let valid = chunk.valid();
            let clean = terminal_text(valid);
            for ((offset, orig), shown) in valid.char_indices().zip(clean.chars()) {
                rows.unit(at + offset, orig, shown);
            }
            at += valid.len();
            for _ in chunk.invalid() {
                rows.unit(at, '\u{fffd}', '\u{fffd}');
                at += 1;
            }
        }
        rows.finish(self.bytes.len())
    }
}

/// The notice a draft past the cap shows in place of itself.
fn over_notice() -> String {
    format!("draft over {CAP} bytes - ^U clears")
}

/// The cells `ch` takes, a non-ASCII character counted as two: ae has no
/// width table, so it errs wide, which breaks a row early and never late.
fn cells(ch: char) -> usize {
    if ch.is_ascii() { 1 } else { 2 }
}

/// The start of `text` that fits `cells`.
fn clip(text: &str, cells_max: usize) -> String {
    let mut used = 0;
    let fits = |ch: &char| {
        used += cells(*ch);
        used <= cells_max
    };
    text.chars().take_while(fits).collect()
}

/// A draft being laid out in rows of at most `area` cells.
#[derive(Default)]
struct Rows {
    area: usize,
    done: Vec<String>,
    line: String,
    used: usize,
    /// Shown units of the current hard-break segment, with draft byte offsets.
    pending: Vec<(usize, char)>,
    /// The byte offset the cursor is at.
    cursor: usize,
    /// The row and text before the cursor, once its offset is reached.
    seen: Option<(usize, String)>,
    /// A `\r` is open: a break unless a `\n` follows it at once.
    cr: bool,
}

impl Rows {
    fn new(area: usize, cursor: usize) -> Self {
        Self {
            area,
            cursor,
            ..Self::default()
        }
    }

    fn end_row(&mut self) {
        self.done.push(std::mem::take(&mut self.line));
        self.used = 0;
    }

    fn mark(&mut self, at: usize) {
        if self.seen.is_none() && at >= self.cursor {
            self.seen = Some((self.done.len(), self.line.clone()));
        }
    }

    /// Finalize rows before locating the cursor: an overflowing prefix ends
    /// at its last space, otherwise at its last fitting character.
    fn flush(&mut self) {
        let units = std::mem::take(&mut self.pending);
        let mut start = 0;
        while start < units.len() {
            let (mut end, mut used, mut space) = (start, 0, None);
            while end < units.len() && used + cells(units[end].1) <= self.area {
                used += cells(units[end].1);
                if units[end].1 == ' ' {
                    space = Some(end + 1);
                }
                end += 1;
            }
            if end < units.len() {
                end = space.unwrap_or(end);
            }
            for &(at, shown) in &units[start..end] {
                self.mark(at);
                self.line.push(shown);
                self.used += cells(shown);
            }
            start = end;
            if start < units.len() {
                self.end_row();
            }
        }
    }

    /// The unit at byte `at`: `orig` as typed, `shown` as drawn. A two-cell one
    /// in a one-cell area is drawn as `?`: no row is wider than its area.
    fn unit(&mut self, at: usize, orig: char, mut shown: char) {
        if std::mem::take(&mut self.cr) && orig != '\n' {
            self.end_row();
        }
        match orig {
            '\r' => {
                self.flush();
                self.mark(at);
                self.cr = true;
            }
            '\n' => {
                self.flush();
                self.mark(at);
                self.end_row();
            }
            _ => {
                if shown == '\t' {
                    shown = ' ';
                }
                if cells(shown) > self.area {
                    shown = '?';
                }
                self.pending.push((at, shown));
            }
        }
    }

    /// The rows, the cursor row and its text before it; a cursor at the end of
    /// a full row is on the empty row after it.
    fn finish(mut self, len: usize) -> (Vec<String>, usize, String) {
        self.flush();
        if std::mem::take(&mut self.cr) {
            self.end_row();
        }
        if self.cursor >= len {
            if self.used + 1 > self.area {
                self.end_row();
            }
            self.mark(self.cursor);
        }
        let last = (self.done.len(), self.line.clone());
        self.done.push(self.line);
        let (row, before) = self.seen.unwrap_or(last);
        (self.done, row, before)
    }
}

/// The rows of a layout that are shown: `start..end`, a marker row each side.
struct Window {
    start: usize,
    end: usize,
    above: bool,
    below: bool,
}

/// Which of `total` rows `cap` shows, the cursor's among them. The tail when
/// it holds the cursor, else the rows around it; a marker costs a row and
/// needs a cap of three, below which the cursor row is shown with what fits.
fn window(total: usize, cursor: usize, cap: usize) -> Window {
    let plain = |start: usize, end: usize| Window {
        start,
        end,
        above: false,
        below: false,
    };
    if total <= cap {
        return plain(0, total);
    }
    if cap < 3 {
        let start = (cursor + 1).saturating_sub(cap);
        return plain(start, start + cap);
    }
    let tail = total - (cap - 1);
    if cursor >= tail {
        return Window {
            above: true,
            ..plain(tail, total)
        };
    }
    let budget = cap - 2;
    let start = cursor.saturating_sub(budget / 2);
    if start == 0 {
        return Window {
            below: true,
            ..plain(0, cap - 1)
        };
    }
    Window {
        above: true,
        below: true,
        ..plain(start, start + budget)
    }
}

/// The terminal's size in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: usize,
    pub height: usize,
}

impl Size {
    /// Assumed while tmux does not say.
    pub const FALLBACK: Self = Self {
        width: 80,
        height: 24,
    };
}

/// The composer as the screen shows it, wrapped for one [`Size`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    /// Every row, the prompt or its indent included.
    pub rows: Vec<String>,
    /// The row the cursor is on.
    pub cursor_row: usize,
    /// That row up to the cursor.
    pub before: String,
    /// The prompt as drawn on the first row: where a screen read back starts.
    pub anchor: String,
}

/// What one entered line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Ask { seat: String, body: String },
    Close(String),
    Refused(String),
}

/// Read one entered line. `@<seat> <body>` asks that lead-pair seat, its body
/// literal whatever it begins with; a bare `/close <id>` withdraws one of this
/// console's asks and any other bare `/word` is refused, so only a seat prefix
/// carries a slash to a seat; anything else asks `pair[0]`, the main seat.
#[must_use]
pub fn command(raw: &[u8], pair: &[String]) -> Command {
    command_to(raw, pair, pair.first().map_or("", String::as_str))
}

/// [`command`] with an unprefixed line asking `speaker`, the seat last addressed.
#[must_use]
pub fn command_to(raw: &[u8], pair: &[String], speaker: &str) -> Command {
    let Ok(text) = std::str::from_utf8(raw) else {
        return Command::Refused("the line is not UTF-8".to_owned());
    };
    if let Some(routed) = text.strip_prefix('@') {
        let (seat, body) = routed
            .split_once(char::is_whitespace)
            .unwrap_or((routed, ""));
        if seat.contains(':') {
            let why = format!("@{seat} names another session; this chat asks its own lead pair");
            return Command::Refused(why);
        }
        if !pair.iter().any(|name| name == seat) {
            return Command::Refused(format!("@{seat} is not a lead-pair seat"));
        }
        return ask(seat, body);
    }
    if text.starts_with('/') {
        let mut words = text.split_whitespace();
        let word = words.next().unwrap_or(text);
        return match (word, words.collect::<Vec<_>>().as_slice()) {
            ("/close", [id]) => Command::Close((*id).to_owned()),
            ("/close", _) => Command::Refused("/close takes exactly one request id".to_owned()),
            _ => Command::Refused(format!(
                "unknown command {word}; to send it, name a seat: @{speaker} {word}"
            )),
        };
    }
    ask(speaker, text)
}

fn ask(seat: &str, body: &str) -> Command {
    if body.trim().is_empty() {
        return Command::Refused(format!("nothing to ask @{seat}"));
    }
    Command::Ask {
        seat: seat.to_owned(),
        body: body.to_owned(),
    }
}

/// One reading of whether this console owns its session's input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    Owner,
    NotOwner(String),
    Unknown,
}

/// What the console does next, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Bracketed paste on or off, written before the line that announces it.
    Paste(bool),
    Print(String),
    /// Lane text whose header, body and outcome keep their line breaks.
    Lane(String),
    /// Submit `raw`, the entered bytes, as an ask of `seat`.
    Ask {
        raw: Vec<u8>,
        seat: String,
        body: String,
    },
    /// Withdraw this console's ask by id.
    Close(String),
}

/// The console's input as a step machine. It composes only while it owns the
/// session's input, and then only keys whose first byte was read after it took
/// ownership: nothing typed at a read-only console — queued, or split across
/// that moment — is ever composed or sent.
#[derive(Debug, Default)]
pub struct Input {
    pair: Vec<String>,
    /// The seat an unprefixed line asks: where `pair` last had an `@seat` ask
    /// go, main until then. Process memory only.
    speaker: usize,
    keys: Keys,
    composer: Composer,
    /// Since when this console takes input; `None` while it does not.
    since: Option<Instant>,
    /// The reason last printed for being read-only, so a tick repeats none.
    read_only: Option<String>,
    /// The terminal's input has ended.
    closed: bool,
}

impl Input {
    /// A read-only input for a lead pair, main seat first.
    #[must_use]
    pub fn new(pair: Vec<String>) -> Self {
        Self {
            pair,
            ..Self::default()
        }
    }

    /// A fresh ownership `reading`, completed at `now`. Only a positive one
    /// promotes, an unknown one changes nothing, and a change drops the draft.
    pub fn tick(&mut self, reading: Reading, now: Instant) -> Vec<Effect> {
        if self.closed {
            return Vec::new();
        }
        match reading {
            Reading::Owner if self.since.is_none() => {
                self.since = Some(now);
                self.composer = Composer::default();
                let mut effects = vec![Effect::Paste(true)];
                if self.read_only.take().is_some() {
                    effects.push(Effect::Print("accepting input".to_owned()));
                }
                effects
            }
            Reading::NotOwner(why) if self.read_only.as_ref() != Some(&why) => {
                self.since = None;
                self.composer = Composer::default();
                let line = format!("read-only: {why} - prefix h opens it");
                self.read_only = Some(why);
                vec![Effect::Paste(false), Effect::Print(line)]
            }
            _ => Vec::new(),
        }
    }

    /// The kept `draft` put back in the composer, literally, and the line saying
    /// it may already be sent: no byte of it is read as a command until a fresh
    /// Enter, and a console that takes no input restores nothing.
    pub fn restore(&mut self, draft: Draft) -> Vec<Effect> {
        if self.since.is_none() {
            return Vec::new();
        }
        let said = match draft {
            Draft::Nothing => return Vec::new(),
            Draft::Kept(bytes) => {
                let _ = self.composer.key(Key::Text(bytes));
                let seats = self.pair.join(", ");
                format!(
                    "Kept line, maybe already sent: check {seats} panes (prefix H) before Enter"
                )
            }
            Draft::Refused(why) => format!("refused: {why}"),
        };
        vec![Effect::Print(said)]
    }

    /// One terminal read, stamped when the read returned.
    pub fn chunk(&mut self, bytes: &[u8], stamp: Instant) -> Vec<Effect> {
        let keys = self.keys.feed(bytes, stamp);
        let Some(since) = self.since else {
            return Vec::new();
        };
        let mut effects = Vec::new();
        for (key, origin) in keys {
            if origin < since {
                continue;
            }
            let effect = match self.composer.key(key) {
                None | Some(Entered::Empty) => continue,
                Some(Entered::Over) => Effect::Print(format!(
                    "refused: the draft is over {CAP} bytes - ^U clears it"
                )),
                Some(Entered::Line(raw)) => match command_to(&raw, &self.pair, self.speaker()) {
                    Command::Ask { seat, body } => {
                        let at = self.pair.iter().position(|name| *name == seat);
                        self.speaker = at.filter(|_| raw.starts_with(b"@")).unwrap_or(self.speaker);
                        Effect::Ask { raw, seat, body }
                    }
                    Command::Close(id) => Effect::Close(id),
                    Command::Refused(why) => Effect::Print(format!("refused: {why}")),
                },
            };
            effects.push(effect);
        }
        effects
    }

    /// The end of the terminal's input: paste off, the draft dropped, and no
    /// key taken again.
    pub fn closed(&mut self) -> Vec<Effect> {
        (self.keys, self.composer) = (Keys::default(), Composer::default());
        (self.since, self.closed) = (None, true);
        let line = "input closed; this chat only reads now".to_owned();
        vec![Effect::Paste(false), Effect::Print(line)]
    }

    /// The seat an unprefixed line asks.
    fn speaker(&self) -> &str {
        self.pair.get(self.speaker).map_or("", String::as_str)
    }

    /// Whether this console takes input.
    #[must_use]
    pub fn taking(&self) -> bool {
        self.since.is_some()
    }

    /// The prompt. The seat name is the meta's, kept verbatim there, so it is
    /// made inert here.
    fn prompt(&self) -> String {
        let speaker = terminal_text(self.speaker());
        format!("to {}> ", speaker.replace(['\n', '\t'], " "))
    }

    /// The composer on one line, while this console takes input.
    #[must_use]
    pub fn line(&self) -> Option<String> {
        self.since.map(|_| self.composer.line(&self.prompt()))
    }

    /// The composer wrapped for a pane of `size`, while this console takes input.
    #[must_use]
    pub fn view(&self, size: Size) -> Option<View> {
        self.since.map(|_| self.composer.view(&self.prompt(), size))
    }
}

/// The one line that says what became of an ask of `seat`.
#[must_use]
pub fn outcome_line(outcome: &Outcome, seat: &str) -> String {
    let check = format!("check {seat} pane (prefix H)");
    match outcome {
        Outcome::Sent(id, None) => format!("sent {id}"),
        Outcome::Sent(id, Some(kept)) => format!("sent {id}; {kept}"),
        Outcome::Uncertain(id) => format!("uncertain {id}: {check}"),
        Outcome::NotDelivered(id) => format!("not delivered {id}"),
        Outcome::Unknown(id, _) => {
            format!("no record of {id}; it may have been delivered - {check}")
        }
    }
}

/// What the last [`paint`] left: where its composer's rows are.
#[derive(Debug, Default)]
pub struct Screen {
    drawn: Option<Drawn>,
}

#[derive(Debug)]
struct Drawn {
    rows: usize,
    /// The rows from the first one down to the one the cursor is on.
    up: usize,
    width: usize,
    anchor: String,
}

/// The terminal's screen as tmux has it now, read once after the pane narrowed.
#[derive(Debug)]
pub struct Seen {
    /// The screen row the cursor is on.
    pub cursor_y: usize,
    /// The visible rows, trailing blanks trimmed.
    pub capture: String,
}

impl Screen {
    /// Whether a composer is on the terminal.
    #[must_use]
    pub fn drawn(&self) -> bool {
        self.drawn.is_some()
    }

    /// Whether erasing at `size` needs the screen read: narrowed, prompt findable.
    #[must_use]
    pub fn needs_seen(&self, size: Size) -> bool {
        let narrowed = |drawn: &Drawn| size.width < drawn.width && trusted(&drawn.anchor);
        self.drawn.as_ref().is_some_and(narrowed)
    }

    /// The write that puts the cursor below the composer, for a line to print
    /// after it; nothing when none is drawn.
    pub fn settle(&mut self) -> String {
        let Some(drawn) = self.drawn.take() else {
            return String::new();
        };
        match drawn.rows.saturating_sub(drawn.up + 1) {
            0 => "\r\n".to_owned(),
            below => format!("\x1b[{below}B\r\n"),
        }
    }
}

/// A prompt long and plain enough to tell from draft text on a screen.
fn trusted(anchor: &str) -> bool {
    anchor.is_ascii() && anchor.trim_end().len() >= 4
}

/// The rows from the composer's first row down to the cursor row, read off
/// `seen`: the prompt as the narrower pane split it, the nearest one at or
/// above the cursor; none means the top is in tmux's history. `None` when the
/// prompt is not [`trusted`].
fn anchor_up(seen: &Seen, anchor: &str, width: usize) -> Option<usize> {
    if !trusted(anchor) {
        return None;
    }
    let pieces: Vec<&str> = anchor
        .as_bytes()
        .chunks(width.max(1))
        .filter_map(|piece| std::str::from_utf8(piece).ok())
        .map(str::trim_end)
        .collect();
    let rows: Vec<&str> = seen.capture.lines().map(str::trim_end).collect();
    let last = pieces.len() - 1;
    let matches = |top: &usize| {
        let at = |piece: usize| rows.get(top + piece).copied().unwrap_or_default();
        let whole = pieces[..last].iter().enumerate();
        whole.into_iter().all(|(piece, text)| at(piece) == *text)
            && at(last).starts_with(pieces[last])
    };
    let top = seen.cursor_y.checked_sub(last);
    let found = top.and_then(|top| (0..=top).rev().find(matches));
    Some(found.map_or(seen.cursor_y, |top| seen.cursor_y - top))
}

/// The write that takes the last draw away: up to its first row, then clear
/// to the end of the screen. Tmux splits a row a narrowed pane cannot hold,
/// so then the way up is read off the screen, else it is as drawn.
fn erase(drawn: &Drawn, size: Size, seen: Option<&Seen>) -> String {
    let read = seen.filter(|_| size.width < drawn.width);
    let up = read
        .and_then(|seen| anchor_up(seen, &drawn.anchor, size.width))
        .unwrap_or(drawn.up);
    match up {
        0 => "\r\x1b[J".to_owned(),
        rise => format!("\x1b[{rise}A\r\x1b[J"),
    }
}

/// The composer's rows, made to exist first by a newline each, which scrolls a
/// composer at the screen's foot, so the cursor saved on its row stays true.
fn draw(view: &View) -> String {
    let below = view.rows.len().saturating_sub(1);
    let mut out = String::from(DRAW_OPEN);
    if below > 0 {
        out.extend(["\n".repeat(below), format!("\x1b[{below}A")]);
    }
    out.push('\r');
    for (at, row) in view.rows.iter().enumerate() {
        if at == view.cursor_row {
            let (head, tail) = row.split_at_checked(view.before.len()).unwrap_or((row, ""));
            out.extend([head, "\x1b7", tail]);
        } else {
            out.push_str(row);
        }
        if at < below {
            out.push_str("\r\n");
        }
    }
    out.extend(["\x1b8", DRAW_CLOSE]);
    out
}

/// One write: the last composer taken away, `text`, each paste switch and
/// printed line in order, then `view` drawn when there is one. `seen` is the
/// screen read after the pane narrowed.
#[must_use]
pub fn paint(
    screen: &mut Screen,
    text: &str,
    effects: &[Effect],
    view: Option<&View>,
    size: Size,
    seen: Option<&Seen>,
) -> String {
    let mut out = match screen.drawn.take() {
        Some(drawn) => erase(&drawn, size, seen),
        None if !text.is_empty() || !effects.is_empty() => "\r\x1b[K".to_owned(),
        None => String::new(),
    };
    out.push_str(text);
    for effect in effects {
        match effect {
            Effect::Paste(on) => out.push_str(if *on { PASTE_ON } else { PASTE_OFF }),
            Effect::Lane(text) => out.push_str(&terminal_text(text)),
            Effect::Print(said) => {
                out.push_str(&terminal_text(said).replace(['\n', '\t'], " "));
                out.push('\n');
            }
            Effect::Ask { .. } | Effect::Close(_) => {}
        }
    }
    if let Some(view) = view {
        out.push_str(&draw(view));
        screen.drawn = Some(Drawn {
            rows: view.rows.len(),
            up: view.cursor_row,
            width: size.width,
            anchor: view.anchor.clone(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        CAP, Command, Composer, Draft, Effect, Entered, Input, Key, Keys, Outcome, Reading, Screen,
        Seen, Size, View, anchor_up, command, command_to, outcome_line, paint, window,
    };
    use std::time::{Duration, Instant};

    const WHY: &str = "input owned by window @2";

    fn size(width: usize, height: usize) -> Size {
        Size { width, height }
    }

    fn drafted(bytes: &[u8]) -> Composer {
        let mut composer = Composer::default();
        let _ = composer.key(Key::Text(bytes.to_vec()));
        composer
    }

    /// The write of `view` on a fresh terminal, the screen it leaves behind.
    fn drawn(input: &Input, size: Size) -> (Screen, String) {
        let mut screen = Screen::default();
        let out = paint(&mut screen, "", &[], input.view(size).as_ref(), size, None);
        (screen, out)
    }

    fn pair() -> Vec<String> {
        vec!["lead".to_owned(), "colead".to_owned()]
    }

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    fn owner(base: Instant) -> Input {
        let mut input = Input::new(pair());
        let _ = input.tick(Reading::Owner, base);
        input
    }

    fn asks(effects: &[Effect]) -> Vec<&str> {
        let mut bodies = Vec::new();
        for effect in effects {
            if let Effect::Ask { body, .. } = effect {
                bodies.push(body.as_str());
            }
        }
        bodies
    }

    /// The keys of `reads`, adjacent text joined whatever its origin.
    fn joined(keys: &mut Keys, reads: &[&[u8]], stamp: Instant) -> Vec<Key> {
        let mut out: Vec<Key> = Vec::new();
        for (key, _) in reads.iter().flat_map(|read| keys.feed(read, stamp)) {
            match (out.last_mut(), key) {
                (Some(Key::Text(run)), Key::Text(more)) => run.extend(more),
                (_, key) => out.push(key),
            }
        }
        out
    }

    #[test]
    fn a_paste_is_one_literal_run_however_its_markers_are_split() {
        let stream = b"x\x1b[200~a\rb\x15\x7f\n\x1b[D\x1b[201~y\r";
        let want = [Key::Text(b"xa\rb\x15\x7f\n\x1b[Dy".to_vec()), Key::Enter];
        let base = Instant::now();
        for cut in 0..=stream.len() {
            let reads = [&stream[..cut], &stream[cut..]];
            assert_eq!(joined(&mut Keys::default(), &reads, base), want, "{cut}");
        }
    }

    #[test]
    fn a_run_is_one_key_and_a_paste_holds_back_only_what_may_still_close_it() {
        let base = Instant::now();
        let stream = b"ab\x1b[200~\x1b\x1b[201~\r\x1b[200~\x1b[X";
        let text = |bytes: &[u8]| (Key::Text(bytes.to_vec()), base);
        let want = [text(b"ab\x1b"), (Key::Enter, base), text(b"\x1b[X")];
        assert_eq!(Keys::default().feed(stream, base), want);
    }

    #[test]
    fn outside_a_paste_escapes_are_consumed_and_one_left_open_is_dropped_whole() {
        let closed = [b"\x1b[".as_slice(), &[b'1'; 13], b"m"].concat();
        let open = [b"\x1b[".as_slice(), &[b'1'; 15]].concat();
        let reads: [&[u8]; 5] = [
            b"\x1b[D\x1b[201~\x1bOAa\x7f\x15",
            &closed,
            &open,
            b"z\r",
            b"\x08",
        ];
        let want = [
            Key::Left,
            Key::Text(b"a".to_vec()),
            Key::Backspace,
            Key::ClearLine,
            Key::Text(b"z".to_vec()),
            Key::Enter,
            Key::Backspace,
        ];
        assert_eq!(joined(&mut Keys::default(), &reads, Instant::now()), want);
    }

    #[test]
    fn every_spelling_of_an_editing_key_is_that_key_however_the_reads_split_it() {
        let keys: [(&[u8], Option<Key>); 22] = [
            (b"\x1b[D", Some(Key::Left)),
            (b"\x1bOD", Some(Key::Left)),
            (b"\x1b[C", Some(Key::Right)),
            (b"\x1bOC", Some(Key::Right)),
            (b"\x1b[H", Some(Key::Home)),
            (b"\x1bOH", Some(Key::Home)),
            (b"\x1b[1~", Some(Key::Home)),
            (b"\x1b[7~", Some(Key::Home)),
            (b"\x01", Some(Key::Home)),
            (b"\x1b[F", Some(Key::End)),
            (b"\x1bOF", Some(Key::End)),
            (b"\x1b[4~", Some(Key::End)),
            (b"\x1b[8~", Some(Key::End)),
            (b"\x05", Some(Key::End)),
            (b"\x1b[3~", Some(Key::Delete)),
            (b"\x1b[1;5D", None),
            (b"\x1b[A", None),
            (b"\x1bOB", None),
            (b"\x1b[2~", None),
            (b"\x1b[5~", None),
            (b"\x1b[6~", None),
            (b"\x1bx", None),
        ];
        for (seq, key) in keys {
            let stream = [b"a", seq, b"b"].concat();
            let want = match key {
                Some(key) => vec![Key::Text(b"a".to_vec()), key, Key::Text(b"b".to_vec())],
                None => vec![Key::Text(b"ab".to_vec())],
            };
            for cut in 0..=stream.len() {
                let reads = [&stream[..cut], &stream[cut..]];
                let got = joined(&mut Keys::default(), &reads, Instant::now());
                assert_eq!(got, want, "{seq:?} cut at {cut}");
            }
        }
    }

    /// `draft` typed, `stream` read as keys, then what Enter sends.
    fn edited(draft: &[u8], stream: &[u8]) -> Vec<u8> {
        let mut composer = drafted(draft);
        for (key, _) in Keys::default().feed(stream, Instant::now()) {
            assert_eq!(composer.key(key), None);
        }
        match composer.key(Key::Enter) {
            Some(Entered::Line(bytes)) => bytes,
            _ => Vec::new(),
        }
    }

    #[test]
    fn text_goes_in_and_comes_out_at_the_cursor_a_unit_at_a_time() {
        let cases: [(&str, &[u8], &[u8]); 8] = [
            (
                "aé中z",
                b"\x1b[D\x1b[D!\x1b[3~\x7f\x1b[C?",
                "aéz?".as_bytes(),
            ),
            ("ab", b"\x01\x7f\x1b[D\x05\x1b[3~\x1b[C!", b"ab!"),
            ("a\nb", b"\x01!\x05?", b"!a\nb?"),
            (
                "a\nb",
                b"\x1b[D\x1b[D\x1b[D!\x1b[C\x1b[C\x1b[C\x1b[C?",
                b"!a\nb?",
            ),
            ("a中", b"\x1b[D\x7f", "中".as_bytes()),
            ("ab", b"\x01\x1b[C\x1b[3~\x1b[3~\x1b[3~", b"a"),
            ("abc", b"\x1b[D\x15x", b"x"),
            ("ab", b"\x1b[D\x1b[200~\x1b[D\x1b[201~!", b"a\x1b[D!b"),
        ];
        for (draft, stream, want) in cases {
            assert_eq!(edited(draft.as_bytes(), stream), want, "{draft:?}");
        }
        let text = |bytes: &[u8]| Key::Text(bytes.to_vec());
        let hostile = |draft: &[u8], keys: Vec<Key>| {
            let mut composer = drafted(draft);
            for key in keys {
                drop(composer.key(key));
            }
            composer.key(Key::Enter)
        };
        let line = |bytes: &[u8]| Some(Entered::Line(bytes.to_vec()));
        let split = vec![Key::Left, text(b"\xe4"), text(b"\xb8\xad"), text(b"!")];
        assert_eq!(
            hostile(b"ab", split),
            line("a中!b".as_bytes()),
            "split in two reads"
        );
        let bad = b"a\xff\xb8";
        assert_eq!(
            hostile(bad, vec![Key::Left, Key::Backspace]),
            line(b"a\xb8")
        );
        let walk = vec![Key::Home, Key::Right, Key::Delete, Key::Delete];
        assert_eq!(hostile(bad, walk), line(b"a"), "each stray byte is a unit");
        // Text goes in contiguously at the byte offset, however the reads split
        // it, even when it completes a character with what follows; only a key
        // that is not text moves the cursor on to the end of that character.
        let (home, stray) = (Key::Home, b"\xa9x");
        let block = hostile(stray, vec![home.clone(), text(b"\xc3\xa9Z")]);
        let parts = hostile(stray, vec![home.clone(), text(b"\xc3"), text(b"\xa9Z")]);
        let want = line(b"\xc3\xa9Z\xa9x");
        assert_eq!((&block, &parts), (&want, &want));
        let past = vec![home.clone(), text(b"\xc3"), Key::Delete, text(b"!")];
        assert_eq!(hostile(stray, past), line(b"\xc3\xa9!"), "Delete after it");
        let whole = hostile(stray, vec![home, text(b"\xc3"), Key::Backspace]);
        assert_eq!(whole, line(b"x"), "Backspace erases it whole");
    }

    #[test]
    fn a_restored_draft_leaves_the_cursor_at_its_end() {
        let base = Instant::now();
        let mut input = owner(base);
        let _ = input.restore(Draft::Kept(b"ab".to_vec()));
        assert_eq!(
            asks(&input.chunk(b"!\x1b[D\x1b[D?\r", at(base, 1))),
            ["a?b!"]
        );
    }

    #[test]
    fn the_cursor_is_where_the_next_byte_goes_on_the_row_it_lands_on() {
        let left = "\x1b[D";
        let view = |draft: &str, stream: &str| {
            let mut composer = drafted(draft.as_bytes());
            for (key, _) in Keys::default().feed(stream.as_bytes(), Instant::now()) {
                let _ = composer.key(key);
            }
            composer.view("to> ", size(10, 24))
        };
        let cases = [
            ("abcdefghij", "\x01".to_owned(), (0, "to> ")),
            ("abcdefghij", left.repeat(4), (1, "    ")),
            ("abcdefghij", left.repeat(5), (0, "to> abcde")),
            ("a\nbc", left.to_string(), (1, "    b")),
            ("a\nbc", left.repeat(3), (0, "to> a")),
            (
                "abcdef\nz",
                format!("\x01{}", "\x1b[C".repeat(6)),
                (0, "to> abcdef"),
            ),
            ("中中中中", left.repeat(2), (0, "to> 中中")),
            ("abcde", String::new(), (0, "to> abcde")),
            ("abcdef", String::new(), (1, "    ")),
        ];
        for (draft, stream, (row, before)) in cases {
            let shown = view(draft, &stream);
            assert_eq!(
                (shown.cursor_row, shown.before.as_str()),
                (row, before),
                "{draft:?}"
            );
        }
        let mut mixed = drafted(b"ab\xffcd");
        for key in [Key::Home, Key::Right, Key::Right, Key::Right, Key::Right] {
            let _ = mixed.key(key);
        }
        assert_eq!(mixed.view("to> ", size(10, 24)).before, "to> ab\u{fffd}c");
        let draft = (0..20)
            .map(|n| format!("l{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let tall = view(&draft, "\x01");
        assert_eq!((tall.cursor_row, tall.rows.len()), (0, 10));
        assert!(
            tall.rows[9].starts_with("    … +11"),
            "a below marker, clipped to the area"
        );
        // A cursor inside the final character, the byte offset text leaves it at,
        // is drawn where the end of that character is: after it, on a row of its own.
        let mut inside = drafted(b"\xb8\xad");
        let _ = (
            inside.key(Key::Home),
            inside.key(Key::Text(b"\xe4".to_vec())),
        );
        let seen = inside.view("", size(2, 24));
        let _ = inside.key(Key::End);
        assert_eq!(seen, inside.view("", size(2, 24)));
        assert_eq!((seen.cursor_row, seen.rows.len()), (1, 2));
    }

    #[test]
    fn an_interior_cursor_is_where_a_paint_climbs_to_and_a_settle_comes_down_from() {
        let base = Instant::now();
        let mut input = owner(base);
        let _ = input.chunk(b"\x1b[200~a\nb\nc\x1b[201~\x1b[H", at(base, 1));
        let big = size(40, 24);
        let (mut screen, first) = drawn(&input, big);
        assert!(
            first.contains("to lead> \x1b7a\r\n"),
            "saved on the cursor row: {first:?}"
        );
        let again = paint(&mut screen, "", &[], input.view(big).as_ref(), big, None);
        assert!(
            again.starts_with("\r\x1b[J"),
            "already on the first row: {again:?}"
        );
        assert_eq!(
            screen.settle(),
            "\x1b[2B\r\n",
            "down the rows below the cursor"
        );
    }

    #[test]
    fn the_composer_holds_exactly_the_cap_and_refuses_past_it_until_cleared() {
        let mut composer = Composer::default();
        let (full, big) = (vec![b'a'; CAP], Key::Text(vec![b'a'; CAP]));
        assert_eq!(composer.key(Key::Text(full.clone())), None);
        assert_eq!(composer.key(Key::Enter), Some(Entered::Line(full)));
        for key in [big, Key::Text(vec![b'b']), Key::Backspace] {
            assert_eq!(composer.key(key), None);
        }
        assert_eq!(composer.key(Key::Enter), Some(Entered::Over));
        let over = "to lead> draft over 65536 bytes - ^U clears";
        assert_eq!(composer.line("to lead> "), over);
        assert_eq!(composer.key(Key::ClearLine), None);
        assert_eq!(composer.key(Key::Enter), Some(Entered::Empty));
        let _ = composer.key(Key::Text("a🎉".as_bytes().to_vec()));
        let _ = composer.key(Key::Backspace);
        assert_eq!(composer.key(Key::Enter), Some(Entered::Line(b"a".to_vec())));
    }
    #[test]
    fn an_over_draft_ignores_text_and_edit_keys_at_a_unit_boundary() {
        let mut composer = drafted(b"ab");
        let _ = (composer.key(Key::Home), composer.key(Key::Right));
        assert_eq!(composer.key(Key::Text(vec![b'x'; CAP])), None);
        let keys = [
            Key::Backspace,
            Key::Delete,
            Key::Left,
            Key::Right,
            Key::Home,
            Key::End,
        ];
        for key in [Key::Text(b"c".to_vec())].into_iter().chain(keys) {
            assert_eq!(composer.key(key), None);
            assert_eq!(
                (composer.bytes.as_slice(), composer.cursor),
                (&b"ab"[..], 1)
            );
        }
        assert_eq!(composer.key(Key::Enter), Some(Entered::Over));
    }

    #[test]
    fn the_composer_line_is_the_whole_draft_on_one_line_with_nothing_live() {
        let line = drafted(b"first\nx\x1b[2J\x07\tb").line("p> ");
        assert_eq!(line, "p> first x\u{fffd}[2J\u{fffd} b");
    }

    #[test]
    fn a_draft_wraps_after_its_prompt_and_every_break_is_a_row() {
        let view = |draft: &[u8], width| drafted(draft).view("to> ", size(width, 24));
        let wrapped = view(b"abcdefghij", 10);
        assert_eq!(wrapped.rows, ["to> abcdef", "    ghij"]);
        assert_eq!(
            (wrapped.cursor_row, wrapped.before.as_str()),
            (1, "    ghij")
        );
        assert_eq!(wrapped.anchor, "to> ");
        let full = view(b"abcdef", 10);
        assert_eq!(full.rows, ["to> abcdef", "    "], "the next byte's row");
        assert_eq!((full.cursor_row, full.before.as_str()), (1, "    "));
        let breaks = view(b"a\nb\r\nc\rd\n", 10);
        assert_eq!(breaks.rows, ["to> a", "    b", "    c", "    d", "    "]);
        assert_eq!(breaks.cursor_row, 4, "CRLF is one break");
        let wide = view("中中中中".as_bytes(), 10);
        assert_eq!(wide.rows, ["to> 中中中", "    中"], "a wide one counts two");
        let odd = view(b"x\ty\xffz\x1b[2J", 40);
        assert_eq!(odd.rows, ["to> x y\u{fffd}z\u{fffd}[2J"]);
        assert_eq!(view(b"", 10).before, "to> ");
    }

    #[test]
    fn at_any_size_no_row_is_wider_than_the_pane_and_the_cursor_row_shows() {
        let mixed = ["a中é\nb\r\n\tz\x07".as_bytes(), b"\xff", &b"q".repeat(40)].concat();
        let drafts = [
            b"".to_vec(),
            mixed,
            "中é".repeat(30).into_bytes(),
            b"x\n".repeat(15),
        ];
        for draft in &drafts {
            for (width, height) in (1..=14).flat_map(|w| [1, 2, 3, 6, 24].map(|h| (w, h))) {
                let view = drafted(draft).view("to lead> ", size(width, height));
                let at = format!("{width}x{height}");
                assert!(
                    view.rows.len() <= 10.min(height.saturating_sub(1).max(1)),
                    "{at}"
                );
                for row in &view.rows {
                    assert!(
                        row.chars().map(super::cells).sum::<usize>() <= width,
                        "{at}"
                    );
                    assert!(!row.contains(|ch: char| ch.is_control()), "{at}");
                }
                let row = view.rows.get(view.cursor_row).expect(&at);
                assert!(row.starts_with(&view.before), "{at}");
            }
        }
    }

    #[test]
    fn the_window_keeps_the_cursor_row_and_spends_a_row_on_each_marker() {
        let cases = [
            ((5, 4, 10), (0, 5, false, false)),
            ((30, 29, 10), (21, 30, true, false)),
            ((30, 15, 10), (11, 19, true, true)),
            ((30, 2, 10), (0, 9, false, true)),
            ((30, 29, 3), (28, 30, true, false)),
            ((30, 15, 3), (15, 16, true, true)),
            ((30, 15, 2), (14, 16, false, false)),
            ((30, 15, 1), (15, 16, false, false)),
            ((30, 0, 2), (0, 2, false, false)),
        ];
        for ((total, cursor, cap), want) in cases {
            let shown = window(total, cursor, cap);
            let got = (shown.start, shown.end, shown.above, shown.below);
            assert_eq!(got, want, "{total} {cursor} {cap}");
        }
    }

    #[test]
    fn a_tall_draft_shows_its_tail_under_a_marker_and_the_pane_height_bounds_it() {
        let draft = (0..20)
            .map(|n| format!("l{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let shown = |height| drafted(draft.as_bytes()).view("to> ", size(40, height));
        let tall = shown(24);
        assert_eq!((tall.rows.len(), tall.cursor_row), (10, 9));
        assert_eq!(
            (tall.rows[0].as_str(), tall.rows[9].as_str()),
            ("to> … +11 lines above", "    l19")
        );
        let short = [
            "to> … +16 lines above",
            "    l16",
            "    l17",
            "    l18",
            "    l19",
        ];
        assert_eq!(shown(6).rows, short);
        assert_eq!(shown(2).rows, ["to> l19"], "one row: the cursor's");
    }

    #[test]
    fn a_draft_past_the_cap_is_one_notice_row_clipped_to_the_pane() {
        let mut composer = drafted(&vec![b'a'; CAP]);
        let _ = composer.key(Key::Text(b"b".to_vec()));
        let rows = |width| composer.view("to lead> ", size(width, 24)).rows;
        assert_eq!(rows(80), ["to lead> draft over 65536 bytes - ^U clears"]);
        assert_eq!(rows(20), ["to lead> draft over "]);
    }

    #[test]
    fn a_line_is_an_ask_of_its_seat_a_close_or_a_named_refusal() {
        let ask = |seat: &str, body: &str| Command::Ask {
            seat: seat.to_owned(),
            body: body.to_owned(),
        };
        let no = |why: &str| Command::Refused(why.to_owned());
        let cases: [(&[u8], Command); 6] = [
            (b"@colead  two\nlines", ask("colead", " two\nlines")),
            (b"/close a b", no("/close takes exactly one request id")),
            (b"@ hi", no("@ is not a lead-pair seat")),
            (b"@colead", no("nothing to ask @colead")),
            (b" \t", no("nothing to ask @lead")),
            (b"\xff", no("the line is not UTF-8")),
        ];
        for (raw, want) in cases {
            let got = command(raw, &pair());
            assert_eq!(got, want, "{}", String::from_utf8_lossy(raw));
        }
    }

    #[test]
    fn an_at_line_that_asks_moves_the_speaker_and_nothing_else_does_but_a_restart() {
        let base = Instant::now();
        let mut input = owner(base);
        let say = |input: &mut Input, line: &[u8]| match input.chunk(line, base).as_slice() {
            [Effect::Ask { seat, body, .. }] => format!("ask {seat} {body}"),
            [Effect::Close(id)] => format!("close {id}"),
            [Effect::Print(said)] => said.clone(),
            other => format!("{other:?}"),
        };
        let cases = [
            ("hi", "ask lead hi", "lead"),
            ("@colead a", "ask colead a", "colead"),
            ("b", "ask colead b", "colead"),
            ("/close x", "close x", "colead"),
            ("@colead", "refused: nothing to ask @colead", "colead"),
            ("@zz q", "refused: @zz is not a lead-pair seat", "colead"),
            (
                "@a:b q",
                "refused: @a:b names another session; this chat asks its own lead pair",
                "colead",
            ),
            (
                "/w",
                "refused: unknown command /w; to send it, name a seat: @colead /w",
                "colead",
            ),
            (" ", "refused: nothing to ask @colead", "colead"),
            ("@lead m", "ask lead m", "lead"),
        ];
        for (line, want, speaker) in cases {
            assert_eq!(
                say(&mut input, &[line.as_bytes(), b"\r"].concat()),
                want,
                "{line}"
            );
            assert_eq!(input.line(), Some(format!("to {speaker}> ")), "{line}");
        }
        let _ = say(&mut input, b"@colead a\r");
        let _ = input.tick(Reading::NotOwner(WHY.to_owned()), base);
        let _ = input.tick(Reading::Owner, base);
        assert_eq!(
            input.line().as_deref(),
            Some("to colead> "),
            "kept across a demotion"
        );
        assert_eq!(
            owner(base).line().as_deref(),
            Some("to lead> "),
            "a restart is main"
        );
        let to_colead = command_to(b"hi", &pair(), "colead");
        assert_eq!(
            to_colead,
            Command::Ask {
                seat: "colead".to_owned(),
                body: "hi".to_owned()
            }
        );
    }

    #[test]
    fn a_restored_draft_is_literal_until_a_fresh_enter_and_only_an_owner_takes_one() {
        let base = Instant::now();
        let kept = |bytes: &[u8]| Draft::Kept(bytes.to_vec());
        let mut idle = Input::new(pair());
        assert_eq!(idle.restore(kept(b"x")), [], "no input, no restore");
        let trio = ["lead", "colead", "third"].map(str::to_owned).to_vec();
        let mut input = Input::new(trio);
        let _ = input.tick(Reading::Owner, base);
        let line = "Kept line, maybe already sent: check lead, colead, third panes (prefix H) before Enter";
        let raw = b"@colead /close x";
        assert_eq!(input.restore(kept(raw)), [Effect::Print(line.to_owned())]);
        assert_eq!(input.line().as_deref(), Some("to lead> @colead /close x"));
        let ask = Effect::Ask {
            raw: raw.to_vec(),
            seat: "colead".to_owned(),
            body: "/close x".to_owned(),
        };
        assert_eq!(input.chunk(b"\r", base), [ask], "the body stays literal");
        assert_eq!(input.line().as_deref(), Some("to colead> "));
        let mut input = owner(base);
        assert_eq!(input.restore(Draft::Nothing), []);
        let why = "the kept draft is over 65536 bytes and was not restored";
        let refused = [Effect::Print(format!("refused: {why}"))];
        assert_eq!(input.restore(Draft::Refused(why.to_owned())), refused);
        assert_eq!(input.line().as_deref(), Some("to lead> "));
        let _ = input.restore(kept(b"\xff"));
        let not_utf8 = [Effect::Print("refused: the line is not UTF-8".to_owned())];
        assert_eq!(input.chunk(b"\r", base), not_utf8);
        let _ = input.restore(kept(&vec![b'a'; CAP]));
        assert_eq!(
            asks(&input.chunk(b"\r", base))[0].len(),
            CAP,
            "exactly the cap"
        );
        let _ = input.restore(kept(&vec![b'a'; CAP + 1]));
        let over = "to lead> draft over 65536 bytes - ^U clears";
        assert_eq!(input.line().as_deref(), Some(over));
    }

    #[test]
    fn ownership_turns_paste_on_before_the_prompt_and_off_before_the_read_only_line() {
        let base = Instant::now();
        let mut input = Input::new(pair());
        assert_eq!(input.tick(Reading::Unknown, base), [], "never promotes");
        let not = || Reading::NotOwner(WHY.to_owned());
        let demoted = || {
            let line = format!("read-only: {WHY} - prefix h opens it");
            vec![Effect::Paste(false), Effect::Print(line)]
        };
        let accepting = Effect::Print("accepting input".to_owned());
        let promoted = [Effect::Paste(true), accepting];
        assert_eq!(input.tick(not(), at(base, 1)), demoted());
        assert_eq!(input.tick(not(), at(base, 2)), []);
        assert_eq!(input.chunk(b"typed\r", at(base, 3)), []);
        assert_eq!(input.line(), None);
        assert_eq!(input.tick(Reading::Owner, at(base, 4)), promoted);
        assert_eq!(input.line().as_deref(), Some("to lead> "));
        assert_eq!(input.chunk(b"half", at(base, 5)), []);
        assert_eq!(
            input.tick(Reading::Owner, at(base, 6)),
            [],
            "still the owner"
        );
        assert_eq!(input.line().as_deref(), Some("to lead> half"));
        assert_eq!(input.tick(not(), at(base, 6)), demoted());
        assert_eq!(input.tick(Reading::Owner, at(base, 7)), promoted);
        assert_eq!(input.chunk(b"\r", at(base, 8)), [], "draft dropped");
        let first = Input::new(pair()).tick(Reading::Owner, base);
        assert_eq!(first, [Effect::Paste(true)]);
        let big = size(80, 24);
        let paint = |text: &str, effects: &[Effect], view: Option<&View>| {
            paint(&mut Screen::default(), text, effects, view, big, None)
        };
        let shown = paint("", &promoted, input.view(big).as_ref());
        let want = "\r\x1b[K\x1b[?2004haccepting input\n\x1b[?7l\rto lead> \x1b7\x1b8\x1b[?7h";
        assert_eq!(shown, want);
        let want = format!("\r\x1b[K\x1b[?2004lread-only: {WHY} - prefix h opens it\n");
        assert_eq!(paint("", &demoted(), None), want);
        assert_eq!(
            (paint("", &[], None), paint("x", &[], None)),
            (String::new(), "\r\x1b[Kx".to_owned())
        );
        assert_eq!(super::RESTORE, "\x1b[?7h\x1b[?2004l");
    }

    #[test]
    fn a_paint_takes_away_exactly_the_rows_it_drew_and_prints_above_them() {
        let base = Instant::now();
        let mut input = owner(base);
        let _ = input.chunk(b"\x1b[200~a\nb\nc\x1b[201~", at(base, 1));
        let big = size(40, 24);
        let (mut screen, first) = drawn(&input, big);
        let rows = "to lead> a\r\n         b\r\n         c";
        assert_eq!(
            first,
            format!("\x1b[?7l\n\n\x1b[2A\r{rows}\x1b7\x1b8\x1b[?7h")
        );
        let effects = [Effect::Paste(true), Effect::Print("hi".to_owned())];
        let again = paint(
            &mut screen,
            "L\n",
            &effects,
            input.view(big).as_ref(),
            big,
            None,
        );
        let want = format!("\x1b[2A\r\x1b[JL\n\x1b[?2004hhi\n{first}");
        assert_eq!(
            again, want,
            "up to the first row, clear, lane, composer again"
        );
        assert_eq!(screen.settle(), "\r\n", "the cursor is on the last row");
        assert_eq!(screen.settle(), "", "and nothing is left to settle");
        let none = paint(&mut drawn(&input, big).0, "", &[], None, big, None);
        assert_eq!(
            none, "\x1b[2A\r\x1b[J",
            "a composer that goes is taken away"
        );
    }

    #[test]
    fn lane_effects_keep_header_body_and_outcome_on_separate_inert_lines() {
        let effects = [Effect::Lane(
            "## ask\n  q\t\x1b[2J\n  sent id\n\n".to_owned(),
        )];
        let out = paint(
            &mut Screen::default(),
            "",
            &effects,
            None,
            size(80, 24),
            None,
        );
        assert_eq!(out, "\r\x1b[K## ask\n  q\t\u{fffd}[2J\n  sent id\n\n");
    }

    #[test]
    fn tabs_and_repeated_spaces_wrap_without_changing_the_entered_bytes() {
        let mut composer = drafted(b"aa\tbb  cc");
        let view = composer.view("", size(5, 24));
        assert_eq!(view.rows, ["aa ", "bb  ", "cc"]);
        assert_eq!((view.cursor_row, view.before.as_str()), (2, "cc"));
        assert_eq!(
            composer.key(Key::Enter),
            Some(Entered::Line(b"aa\tbb  cc".to_vec()))
        );
    }

    #[test]
    fn a_seat_name_from_the_meta_reaches_the_prompt_inert() {
        let mut input = Input::new(vec!["le\x1b[2Jad\x07\n".to_owned()]);
        let _ = input.tick(Reading::Owner, Instant::now());
        let inert = "to le\u{fffd}[2Jad\u{fffd} > ";
        assert_eq!(input.line().as_deref(), Some(inert));
    }

    #[test]
    fn nothing_begun_before_ownership_is_composed_however_the_reads_split_it() {
        let base = Instant::now();
        let mut input = Input::new(pair());
        let _ = input.tick(Reading::Owner, at(base, 5));
        assert_eq!(input.chunk(b"x\r", at(base, 4)), [], "read before");
        assert_eq!(asks(&input.chunk(b"y\r", at(base, 6))), ["y"]);
        let stream = b"\x1b[200~ab\r\x1b[201~";
        for cut in 1..=stream.len() {
            let mut input = Input::new(pair());
            assert_eq!(input.chunk(&stream[..cut], at(base, 1)), [], "{cut}");
            let _ = input.tick(Reading::Owner, at(base, 2));
            assert_eq!(input.chunk(&stream[cut..], at(base, 3)), [], "{cut}");
            assert_eq!(asks(&input.chunk(b"e\r", at(base, 4))), ["e"], "{cut}");
            let mut control = owner(base);
            let _ = control.chunk(&stream[..cut], at(base, 1));
            let _ = control.chunk(&stream[cut..], at(base, 2));
            assert_eq!(asks(&control.chunk(b"\r", at(base, 3))), ["ab\r"], "{cut}");
        }
    }

    #[test]
    fn an_overflowing_paste_submits_nothing_until_it_is_cleared() {
        let base = Instant::now();
        let mut input = owner(base);
        let tail: &[u8] = b"\x15x\ry\nz\r\n\x1b[201~";
        let paste = [b"\x1b[200~".as_slice(), &vec![b'a'; CAP], tail].concat();
        for read in paste.chunks(4096) {
            assert_eq!(input.chunk(read, base), []);
        }
        let refused = "refused: the draft is over 65536 bytes - ^U clears it".to_owned();
        assert_eq!(input.chunk(b"\r", at(base, 1)), [Effect::Print(refused)]);
        assert_eq!(asks(&input.chunk(b"\x15x\r", at(base, 2))), ["x"]);
    }

    #[test]
    fn the_end_of_input_ends_a_paste_and_nothing_is_taken_after_it() {
        let base = Instant::now();
        let mut input = owner(base);
        assert_eq!(input.chunk(b"\x1b[200~half\r", at(base, 1)), []);
        let closed = "input closed; this chat only reads now".to_owned();
        assert_eq!(
            input.closed(),
            [Effect::Paste(false), Effect::Print(closed)]
        );
        assert_eq!(input.line(), None);
        assert_eq!(input.tick(Reading::Owner, at(base, 2)), []);
        assert_eq!(input.chunk(b"x\r", at(base, 3)), []);
    }

    #[test]
    fn after_a_narrowing_the_way_up_is_the_prompt_found_on_the_screen() {
        let up = |cursor_y, capture: &str, anchor, width| {
            let seen = Seen {
                cursor_y,
                capture: capture.to_owned(),
            };
            anchor_up(&seen, anchor, width)
        };
        let split = "L1\nto le\nad> ab\n     cd\n     ef";
        assert_eq!(up(4, split, "to lead> ", 5), Some(3), "the prompt, split");
        assert_eq!(
            up(1, "ad> ab\n     cd", "to lead> ", 5),
            Some(1),
            "in history"
        );
        let ones = "t\no\n\nl\ne\na\nd\n>";
        assert_eq!(
            up(8, ones, "to lead> ", 1),
            Some(8),
            "a cell wide, blanks read"
        );
        let spelt = "to le\nad> x\nto le\nad> y";
        assert_eq!(
            up(3, spelt, "to lead> ", 5),
            Some(1),
            "L1: a draft spelling it"
        );
        assert_eq!(
            up(3, spelt, "to> ", 2),
            None,
            "too short to tell from a draft"
        );
        assert_eq!(up(3, spelt, "é> abc", 5), None, "not plain ASCII");
    }

    /// `draft` drawn at `from` columns, the pane narrowed to `to` and painted
    /// over `capture`: the screen read, the row erased from, what Enter sends.
    fn narrowed(
        draft: &[u8],
        (from, to): (usize, usize),
        capture: &str,
        cursor_y: usize,
    ) -> (bool, usize, Vec<u8>) {
        let base = Instant::now();
        let mut input = owner(base);
        let _ = input.chunk(draft, at(base, 1));
        let (mut screen, _) = drawn(&input, size(from, 24));
        let read = screen.needs_seen(size(to, 24));
        let seen = Seen {
            cursor_y,
            capture: capture.to_owned(),
        };
        let view = input.view(size(to, 24));
        let out = paint(
            &mut screen,
            "",
            &[],
            view.as_ref(),
            size(to, 24),
            Some(&seen),
        );
        let up = out
            .strip_prefix("\x1b[")
            .and_then(|rest| rest.split_once('A'));
        let up = up.map_or(0, |(rows, _)| rows.parse().expect("rows up"));
        let sent = match input.chunk(b"\r", at(base, 2)).as_slice() {
            [Effect::Ask { raw, .. }] => raw.clone(),
            other => panic!("{other:?}"),
        };
        (read, cursor_y - up, sent)
    }

    #[test]
    fn l1_a_composer_drawn_in_six_columns_leaves_stale_rows_under_the_lane_and_the_draft_whole() {
        let capture = "vvv\nto \nabc\n   \ndef\n   \nghi\n   \nj";
        let (read, from, sent) = narrowed(b"abcdefghij", (6, 3), capture, 8);
        assert!(!read, "its prompt is `to `, too short to find");
        assert_eq!(from, 5, "rows 1 to 4 stay, the lane row 0 is never cleared");
        assert_eq!(sent, b"abcdefghij", "a fresh Enter sends the whole draft");
    }

    #[test]
    fn l1_a_draft_spelling_the_prompt_leaves_stale_rows_under_the_lane_and_the_draft_whole() {
        let capture = "lane\nto le\nad> x\nto le\nad> z";
        let (read, from, sent) = narrowed(b"xto lead> z", (40, 5), capture, 4);
        assert!(read, "a prompt long enough to find");
        assert_eq!(from, 3, "the draft's copy of it, not the real top at row 1");
        assert_eq!(sent, b"xto lead> z", "a fresh Enter sends the whole draft");
        let (screen, _) = drawn(&owner(Instant::now()), size(40, 24));
        assert!(!screen.needs_seen(size(40, 24)) && !screen.needs_seen(size(60, 24)));
    }

    #[test]
    fn a_screen_is_drawn_from_its_first_composer_until_it_is_settled() {
        assert!(!Screen::default().drawn());
        let (mut screen, _) = drawn(&owner(Instant::now()), size(40, 24));
        assert!(screen.drawn());
        let _ = screen.settle();
        assert!(!screen.drawn());
    }

    #[test]
    fn a_pane_that_did_not_narrow_is_erased_as_drawn_whatever_the_screen_says() {
        let (read, from, _) = narrowed(b"abcdefghij", (20, 20), "to lead> x\nlane\nlane", 2);
        assert!(!read, "no screen read was owed");
        assert_eq!(from, 2, "one row drawn: no climb, the rows above are lane");
    }

    #[test]
    fn every_outcome_is_one_line_naming_its_request() {
        let id = || "ae-1".to_owned();
        let kept = Some("the draft is kept (denied)".to_owned());
        let check = "check lead pane (prefix H)";
        for (outcome, line) in [
            (
                Outcome::Sent(id(), kept),
                "sent ae-1; the draft is kept (denied)".to_owned(),
            ),
            (Outcome::Uncertain(id()), format!("uncertain ae-1: {check}")),
            (Outcome::NotDelivered(id()), "not delivered ae-1".to_owned()),
            (
                Outcome::Unknown(id(), "said".to_owned()),
                format!("no record of ae-1; it may have been delivered - {check}"),
            ),
        ] {
            assert_eq!(outcome_line(&outcome, "lead"), line);
        }
    }
}
