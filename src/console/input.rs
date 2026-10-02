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
/// How many cells of the draft's tail the composer line shows.
pub const TAIL_CELLS: usize = 72;
/// Bracketed paste on: the terminal marks every paste.
pub const PASTE_ON: &str = "\x1b[?2004h";
/// Bracketed paste off.
pub const PASTE_OFF: &str = "\x1b[?2004l";
/// Opens every composer draw: line start, cleared, autowrap off.
pub const DRAW_OPEN: &str = "\r\x1b[K\x1b[?7l";
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
                    self.pending.clear();
                    self.begun = None;
                }
            } else {
                let key = match byte {
                    0x7f | 0x08 => Key::Backspace,
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

/// What Enter made of the composed bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entered {
    Line(Vec<u8>),
    Over,
    Empty,
}

/// One composed line: literal bytes up to [`CAP`]. Past the cap it is over —
/// later bytes are dropped and Enter refuses — until ^U clears it.
#[derive(Debug, Default)]
pub struct Composer {
    bytes: Vec<u8>,
    over: bool,
}

impl Composer {
    /// Take one key; `Some` for Enter alone.
    pub fn key(&mut self, key: Key) -> Option<Entered> {
        match key {
            Key::Text(text) if !self.over => {
                if self.bytes.len() + text.len() > CAP {
                    self.over = true;
                } else {
                    self.bytes.extend(text);
                }
            }
            Key::Backspace if !self.over => {
                while let Some(byte) = self.bytes.pop() {
                    if byte & 0xc0 != 0x80 {
                        break;
                    }
                }
            }
            Key::Text(_) | Key::Backspace => {}
            Key::ClearLine => *self = Self::default(),
            Key::Enter if self.over => return Some(Entered::Over),
            Key::Enter if self.bytes.is_empty() => return Some(Entered::Empty),
            Key::Enter => return Some(Entered::Line(std::mem::take(&mut self.bytes))),
        }
        None
    }

    /// `prompt`, then the tail of the draft's last line — never a byte a
    /// terminal would act on — or, past the cap, the notice saying so.
    #[must_use]
    pub fn line(&self, prompt: &str) -> String {
        if self.over {
            return format!("{prompt}draft over {CAP} bytes - ^U clears");
        }
        let draft = String::from_utf8_lossy(&self.bytes);
        let last = draft.rsplit(['\n', '\r']).next().unwrap_or_default();
        let clean = terminal_text(last).replace('\t', " ");
        format!("{prompt}{}", tail(&clean, TAIL_CELLS))
    }
}

/// The end of `text` that fits `cells`, a non-ASCII character counted as two:
/// ae has no width table, so it errs wide rather than wrap.
fn tail(text: &str, cells: usize) -> &str {
    let (mut used, mut start) = (0, text.len());
    for (at, ch) in text.char_indices().rev() {
        used += if ch.is_ascii() { 1 } else { 2 };
        if used > cells {
            break;
        }
        start = at;
    }
    &text[start..]
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
            let why = format!("@{seat} names another session; this console asks its own lead pair");
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
        let line = "input closed; this console only reads now".to_owned();
        vec![Effect::Paste(false), Effect::Print(line)]
    }

    /// The seat an unprefixed line asks.
    fn speaker(&self) -> &str {
        self.pair.get(self.speaker).map_or("", String::as_str)
    }

    /// The composer line, while this console takes input. The seat name is
    /// the meta's, kept verbatim there, so it is made inert here.
    #[must_use]
    pub fn line(&self) -> Option<String> {
        let speaker = terminal_text(self.speaker());
        let prompt = format!("to {}> ", speaker.replace(['\n', '\t'], " "));
        self.since.map(|_| self.composer.line(&prompt))
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

/// One write: the composer line cleared, `text`, each paste switch and printed
/// line in order, then the composer line drawn again when there is one.
#[must_use]
pub fn paint(text: &str, effects: &[Effect], line: Option<&str>) -> String {
    let mut out = String::new();
    if !text.is_empty() || !effects.is_empty() {
        out.push_str("\r\x1b[K");
    }
    out.push_str(text);
    for effect in effects {
        match effect {
            Effect::Paste(on) => out.push_str(if *on { PASTE_ON } else { PASTE_OFF }),
            Effect::Print(said) => {
                out.push_str(&terminal_text(said).replace(['\n', '\t'], " "));
                out.push('\n');
            }
            Effect::Ask { .. } | Effect::Close(_) => {}
        }
    }
    if let Some(line) = line {
        out.extend([DRAW_OPEN, line, DRAW_CLOSE]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        CAP, Command, Composer, Draft, Effect, Entered, Input, Key, Keys, Outcome, Reading,
        command, command_to, outcome_line, paint,
    };
    use std::time::{Duration, Instant};

    const WHY: &str = "input owned by window @2";

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
    fn the_composer_line_is_the_clipped_tail_of_the_last_line_with_nothing_live() {
        let mut composer = Composer::default();
        let _ = composer.key(Key::Text(b"first\nx\x1b[2J\x07\tb".to_vec()));
        assert_eq!(composer.line("p> "), "p> x\u{fffd}[2J\u{fffd} b");
        let mut long = Composer::default();
        let text = [vec![b'a'; 100], "é".repeat(3).into_bytes()].concat();
        let _ = long.key(Key::Text(text));
        let want = format!("p> {}{}", "a".repeat(66), "é".repeat(3));
        assert_eq!(long.line("p> "), want, "72 cells, each é counted as two");
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
                "refused: @a:b names another session; this console asks its own lead pair",
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
        let shown = paint("", &promoted, input.line().as_deref());
        let want = "\r\x1b[K\x1b[?2004haccepting input\n\r\x1b[K\x1b[?7lto lead> \x1b[?7h";
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
        let closed = "input closed; this console only reads now".to_owned();
        assert_eq!(
            input.closed(),
            [Effect::Paste(false), Effect::Print(closed)]
        );
        assert_eq!(input.line(), None);
        assert_eq!(input.tick(Reading::Owner, at(base, 2)), []);
        assert_eq!(input.chunk(b"x\r", at(base, 3)), []);
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
