//! The pinned Telegram board: one message in the configured chat that shows
//! the orchestrator's board and is edited in place when the board changes.
//!
//! [`Machine::pass`] makes the calls the board needs, in this order and each
//! independent of the others' failure: the send of a message that does not
//! exist yet, the edit of one that does not show the board, the pin of one that
//! is not pinned and whose id is on disk. Each kind has its own in-memory
//! backoff, so a pin that is refused never holds an edit back and an edit that
//! is due every pass never holds a pin back. A send or an edit waits 1 s after
//! a failure and doubles to a minute; a pin waits 40 s and doubles to ten
//! minutes; a longer `retry_after` wins. A refusal keeps the message id: only an
//! edit that Telegram answers with `message to edit not found` clears it, and
//! that pass ends there, so the next pass sends a new message.
//!
//! Two files in the bridge's state directory carry what a restart must know.
//! `desk-pin` holds the chat, the message id and whether it is pinned. It is
//! checkpointed whole and read strictly: a file that does not parse HOLDS the
//! board, with no call at all, until an operator repairs or removes it, so a
//! damaged id never becomes a second message. `desk-pin.shown` holds the text
//! the message shows, bound to its chat and message id, so an unchanged board
//! costs no call after a restart. It is advisory: a missing, damaged or
//! mismatched sidecar costs one edit, which Telegram answers with "not
//! modified". A message Telegram accepted whose checkpoint cannot be written
//! stays in memory, and no second message is sent while the disk is repaired.

use std::fmt::{self, Write as _};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::{Api, DeskFailure, backoff_delay, durable_write};
use crate::desk::{Desk, Item, Section};
use crate::store::{self, SourceRead};

#[cfg(test)]
mod unit_tests;

/// The checkpoint: chat, message id, pinned or not.
pub(super) const PIN_FILE: &str = "desk-pin";
/// The sidecar: the text the message shows.
pub(super) const SHOWN_FILE: &str = "desk-pin.shown";

const PIN_TAG: &str = "ae-desk-pin 1";
const SHOWN_TAG: &str = "ae-desk-shown 1";
/// The most bytes a checkpoint may have.
const PIN_CAP: u64 = 512;
/// The most bytes a sidecar may have: its header and a rendered board of
/// [`BUDGET`] units of at most four bytes each.
const SHOWN_CAP: usize = 16_384;
/// The longest sidecar header: its tag and the chat and message lines.
const SHOWN_HEADER: usize = 128;
const _: () = assert!(SHOWN_CAP >= SHOWN_HEADER + 4 * BUDGET);
/// The most UTF-16 units of markup one board message carries. Telegram's own
/// limit is 4096 on the text after the markup is parsed, so this is an upper
/// bound with room to spare. A unit is at least one scalar, and a scalar is at
/// most four bytes.
const BUDGET: usize = 3900;
/// What the `… N more: /board` line may need.
const TRAILER_ROOM: usize = 24;

// ─── the checkpoint ──────────────────────────────────────────────────────

/// The message the board lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    /// Telegram's id of the message.
    pub message: i64,
    /// Whether the pin call for it was accepted.
    pub pinned: bool,
}

/// What `desk-pin` records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    /// The chat the message is in, as configured.
    pub chat: String,
    /// The message, once one was sent.
    pub placed: Option<Placed>,
}

impl Pin {
    /// The whole file, in the one form [`parse_pin`] accepts.
    #[must_use]
    pub fn encode(&self) -> String {
        match self.placed {
            None => format!("{PIN_TAG}\nchat {}\n", self.chat),
            Some(placed) => format!(
                "{PIN_TAG}\nchat {}\nmessage {}\npinned {}\n",
                self.chat,
                placed.message,
                u8::from(placed.pinned)
            ),
        }
    }
}

/// A chat id as a checkpoint line may carry it: 1 to 64 bytes of letters,
/// digits, `@`, `_` and `-`. That holds a numeric id (`-100...`) and a
/// `@channelname`, and nothing that could break a line.
fn valid_chat(chat: &str) -> bool {
    (1..=64).contains(&chat.len())
        && chat
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'@' | b'_' | b'-'))
}

fn chat_line(line: &str) -> Result<&str, &'static str> {
    line.strip_prefix("chat ")
        .filter(|chat| valid_chat(chat))
        .ok_or("the chat line is not `chat <id>`")
}

/// A message id as a checkpoint line writes it: every positive `i64`, in
/// decimal digits without a leading zero (at most 19 of them).
fn message_line(line: &str) -> Result<i64, &'static str> {
    let digits = line
        .strip_prefix("message ")
        .filter(|digits| {
            (1..=19).contains(&digits.len())
                && digits.bytes().all(|byte| byte.is_ascii_digit())
                && !digits.starts_with('0')
        })
        .ok_or("the message line is not `message <id>`")?;
    digits
        .parse()
        .map_err(|_| "the message line is not `message <id>`")
}

/// Parse `desk-pin` strictly: the one form [`Pin::encode`] writes, nothing
/// else. Hostile or hand-edited bytes are an `Err`, never a guess.
///
/// # Errors
///
/// Why the bytes are no checkpoint.
pub fn parse_pin(bytes: &[u8]) -> Result<Pin, &'static str> {
    if u64::try_from(bytes.len()).is_ok_and(|len| len > PIN_CAP) {
        return Err("the checkpoint is too long");
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "the checkpoint is not UTF-8")?;
    let body = text
        .strip_suffix('\n')
        .ok_or("the checkpoint has no final line break")?;
    let lines: Vec<&str> = body.split('\n').collect();
    match lines.as_slice() {
        [PIN_TAG, chat] => Ok(Pin {
            chat: chat_line(chat)?.to_owned(),
            placed: None,
        }),
        [PIN_TAG, chat, message, pinned] => {
            let pinned = match *pinned {
                "pinned 0" => false,
                "pinned 1" => true,
                _ => return Err("the pinned line is not `pinned 0` or `pinned 1`"),
            };
            Ok(Pin {
                chat: chat_line(chat)?.to_owned(),
                placed: Some(Placed {
                    message: message_line(message)?,
                    pinned,
                }),
            })
        }
        _ => Err("not a desk-pin checkpoint"),
    }
}

/// What reading the checkpoint found.
enum PinRead {
    Absent,
    Found(Pin),
    /// Something stands there that is not a checkpoint, and why.
    Hold(String),
}

fn read_pin(dir: &Path) -> PinRead {
    match store::read_capped_in(dir, PIN_FILE, PIN_CAP) {
        Err(store::Oversized) => PinRead::Hold(format!("larger than {PIN_CAP} bytes")),
        Ok(SourceRead::Absent) => PinRead::Absent,
        Ok(SourceRead::Invalid(why)) => PinRead::Hold(why),
        Ok(SourceRead::Unreadable(why)) => PinRead::Hold(format!("unreadable: {why}")),
        Ok(SourceRead::Ready(bytes)) => match parse_pin(&bytes) {
            Ok(pin) => PinRead::Found(pin),
            Err(why) => PinRead::Hold(why.to_owned()),
        },
    }
}

/// What `ae telegram status` says about the board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinStatus {
    /// No checkpoint: the board was never sent.
    Absent,
    /// A chat is recorded and no message was sent.
    NoMessage {
        /// The recorded chat.
        chat: String,
    },
    /// A message was sent and its pin is not accepted yet.
    Pending {
        /// The message.
        message: i64,
    },
    /// The message is sent and pinned.
    Pinned {
        /// The message.
        message: i64,
    },
    /// The checkpoint cannot be read, and the board waits for an operator.
    Hold {
        /// Why it cannot be read.
        why: String,
        /// The file to repair or remove.
        path: PathBuf,
    },
}

impl fmt::Display for PinStatus {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => out.write_str("none yet"),
            Self::NoMessage { chat } => write!(out, "no message yet (chat {chat})"),
            Self::Pending { message } => write!(out, "message {message} sent, pin pending"),
            Self::Pinned { message } => write!(out, "message {message} pinned"),
            Self::Hold { why, path } => {
                write!(out, "HOLD - {why}; repair or remove {}", path.display())
            }
        }
    }
}

/// The board's checkpoint in the bridge state directory `dir`, read and
/// nothing else: a status never writes.
#[must_use]
pub fn pin_status(dir: &Path) -> PinStatus {
    match read_pin(dir) {
        PinRead::Absent => PinStatus::Absent,
        PinRead::Found(Pin { chat, placed: None }) => PinStatus::NoMessage { chat },
        PinRead::Found(Pin {
            placed: Some(Placed { message, pinned }),
            ..
        }) => {
            if pinned {
                PinStatus::Pinned { message }
            } else {
                PinStatus::Pending { message }
            }
        }
        PinRead::Hold(why) => PinStatus::Hold {
            why,
            path: dir.join(PIN_FILE),
        },
    }
}

// ─── the sidecar ─────────────────────────────────────────────────────────

/// The text a message shows, with the chat and message it belongs to.
#[derive(Debug, Clone)]
struct Shown {
    chat: String,
    message: i64,
    text: String,
}

impl Shown {
    fn encode(&self) -> String {
        encode_shown(&self.chat, self.message, &self.text)
    }
}

/// The whole sidecar for `text`, bound to `chat` and `message`.
#[must_use]
pub fn encode_shown(chat: &str, message: i64, text: &str) -> String {
    format!("{SHOWN_TAG}\nchat {chat}\nmessage {message}\n{text}")
}

/// The text of a sidecar, when it names exactly `chat` and `message`.
#[must_use]
pub fn parse_shown<'a>(text: &'a str, chat: &str, message: i64) -> Option<&'a str> {
    let rest = text.strip_prefix(SHOWN_TAG)?.strip_prefix('\n')?;
    let rest = rest
        .strip_prefix("chat ")?
        .strip_prefix(chat)?
        .strip_prefix('\n')?;
    let (id, body) = rest.strip_prefix("message ")?.split_once('\n')?;
    (id == message.to_string()).then_some(body)
}

// ─── the rendering ───────────────────────────────────────────────────────

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for found in text.chars() {
        match found {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    out
}

fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

/// One item as HTML: its id and text, then each link on a line of its own, a
/// web link as an anchor and a path as code.
fn item_html(item: &Item) -> String {
    let mut out = format!("{} {}", item.id, escape(&item.text));
    for link in &item.links {
        let shown = escape(link);
        if link.starts_with('/') {
            let _ = write!(out, "\n  <code>{shown}</code>");
        } else {
            let _ = write!(out, "\n  <a href=\"{shown}\">{shown}</a>");
        }
    }
    out
}

/// The board as one Telegram HTML message of at most [`BUDGET`] units: the
/// header, then each section's heading and items in order. Whole items are
/// left out from the tail, so the last section goes first, and a line says how
/// many and where to see them.
#[must_use]
pub(super) fn render(desk: &Desk) -> String {
    let stamp = desk.updated.to_string();
    let clock = stamp.get(11..16).unwrap_or("--:--");
    let mut text = format!("<b>BOARD</b>  updated {clock} UTC");
    let mut shown = 0;
    let mut fits = true;
    for section in Section::ALL {
        let mut first = true;
        for item in desk.items.iter().filter(|item| item.id.section == section) {
            if !fits {
                break;
            }
            let mut block = String::new();
            if first {
                let _ = write!(block, "\n\n<b>{}</b>", section.heading());
            }
            block.push('\n');
            block.push_str(&item_html(item));
            if units(&text) + units(&block) + TRAILER_ROOM > BUDGET {
                fits = false;
                break;
            }
            text.push_str(&block);
            first = false;
            shown += 1;
        }
    }
    let omitted = desk.items.len() - shown;
    if omitted > 0 {
        let _ = write!(text, "\n… {omitted} more: /board");
    }
    text
}

// ─── the machine ─────────────────────────────────────────────────────────

/// What one pass did, for the caller that must know the board is held.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Pass {
    /// Why no call was made: the checkpoint cannot be read.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "the acceptance specs read the hold")
    )]
    pub(super) held: Option<String>,
}

/// The seconds a send or an edit waits after `failures` in a row.
fn edit_delay(failures: u32) -> u64 {
    backoff_delay(failures).as_secs()
}

/// The first wait after a refused pin, in seconds. A pin is not urgent and a
/// refusal is usually a missing right, so its retries are spaced far wider.
const PIN_FIRST: u64 = 40;
/// The longest wait between pin attempts: reached on the fifth failure.
const PIN_MAX: u64 = 600;

/// The seconds a pin waits after `failures` in a row.
fn pin_delay(failures: u32) -> u64 {
    if failures == 0 {
        return 0;
    }
    PIN_FIRST
        .saturating_mul(1_u64 << failures.saturating_sub(1).min(10))
        .min(PIN_MAX)
}

/// When a kind of call may be made again.
#[derive(Debug)]
struct Backoff {
    delay: fn(u32) -> u64,
    failures: u32,
    /// The first second a call is allowed.
    next: i64,
}

impl Backoff {
    const fn new(delay: fn(u32) -> u64) -> Self {
        Self {
            delay,
            failures: 0,
            next: 0,
        }
    }

    const fn due(&self, now: i64) -> bool {
        now >= self.next
    }

    fn reset(&mut self) {
        self.failures = 0;
        self.next = 0;
    }

    /// Note a failure that asked to wait `asked` seconds: the longer of that
    /// and the doubling schedule applies.
    fn fail(&mut self, now: i64, asked: u64) {
        self.failures = self.failures.saturating_add(1);
        let wait = (self.delay)(self.failures).max(asked);
        self.next = now.saturating_add(i64::try_from(wait).unwrap_or(i64::MAX));
    }
}

/// The seconds a failure asked to wait.
const fn asked(failure: DeskFailure) -> u64 {
    match failure {
        DeskFailure::RetryAfter(seconds) => seconds,
        _ => 0,
    }
}

/// The board's message as one pass found it.
struct Live {
    pin: Pin,
    shown: Option<String>,
}

/// Keeps the board message in step with the board.
pub(super) struct Machine {
    dir: PathBuf,
    send: Backoff,
    edit: Backoff,
    pin: Backoff,
    /// A checkpoint Telegram's answer produced and the disk refused.
    unsaved_pin: Option<Pin>,
    unsaved_shown: Option<Shown>,
    /// Whether the line about a running namesake of the seat was told.
    namesake_told: bool,
    /// The line last told, so a state that lasts is told once.
    said: Option<String>,
}

impl Machine {
    /// A machine keeping its checkpoints in the bridge state directory `dir`.
    pub(super) const fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            send: Backoff::new(edit_delay),
            edit: Backoff::new(edit_delay),
            pin: Backoff::new(pin_delay),
            unsaved_pin: None,
            unsaved_shown: None,
            namesake_told: false,
            said: None,
        }
    }

    fn tell(&mut self, err: &mut impl Write, line: String) {
        if self.said.as_deref() != Some(line.as_str()) {
            let _ = writeln!(err, "ae: telegram: {line}");
            self.said = Some(line);
        }
    }

    /// The board checkpoint cannot be read: the one line that says so, and
    /// the hold that goes with it.
    fn hold(&mut self, why: String, err: &mut impl Write) -> Pass {
        let path = self.dir.join(PIN_FILE);
        self.tell(
            err,
            format!(
                "desk pin state {} unreadable ({why}): holding, nothing is sent and the pin \
                 keeps its last text; repair or remove the file",
                path.display()
            ),
        );
        Pass { held: Some(why) }
    }

    /// A running session has the seat's name and is not the seat: told once per
    /// episode, and the board waits for the seat itself.
    pub(super) fn namesake(
        &mut self,
        name: &str,
        running: &Path,
        held: &Path,
        err: &mut impl Write,
    ) {
        if !self.namesake_told {
            self.namesake_told = true;
            let _ = writeln!(
                err,
                "ae: telegram: desk skipped: running session {name} is at {}, not the census-held {}",
                running.display(),
                held.display()
            );
        }
    }

    /// The episode of a running namesake is over: the next one is told again.
    pub(super) fn namesake_over(&mut self) {
        self.namesake_told = false;
    }

    /// The board cannot be read, so the message keeps what it shows. A hold
    /// on the checkpoint is told first: it is the one an operator must act on.
    pub(super) fn unreadable(&mut self, chat: &str, why: &str, err: &mut impl Write) {
        if let Err(held) = self.live(chat, err) {
            let _ = self.hold(held, err);
            return;
        }
        self.tell(
            err,
            format!("the board is unreadable ({why}); the pin keeps its last text"),
        );
    }

    /// Checkpoint `pin`. A refusal keeps it in memory, and is told: until the
    /// disk takes the message id, no pin is attempted.
    fn save_pin(&mut self, pin: &Pin, err: &mut impl Write) {
        match durable_write(&self.dir.join(PIN_FILE), &pin.encode()) {
            Ok(()) => self.unsaved_pin = None,
            Err(why) => {
                let message = pin.placed.map_or_else(
                    || "the board checkpoint".to_owned(),
                    |placed| format!("the checkpoint of board message {}", placed.message),
                );
                self.unsaved_pin = Some(pin.clone());
                self.tell(
                    err,
                    format!("{message} was not written ({why}); the pin waits until it is"),
                );
            }
        }
    }

    /// Checkpoint the shown text. A refusal keeps it in memory and costs
    /// nothing else: the text is a cache.
    fn save_shown(&mut self, shown: &Shown) {
        match durable_write(&self.dir.join(SHOWN_FILE), &shown.encode()) {
            Ok(()) => self.unsaved_shown = None,
            Err(_) => self.unsaved_shown = Some(shown.clone()),
        }
    }

    /// The text `message` in `chat` is known to show.
    fn shown_text(&mut self, chat: &str, message: i64) -> Option<String> {
        if let Some(shown) = self.unsaved_shown.take()
            && shown.chat == chat
            && shown.message == message
        {
            self.save_shown(&shown);
            return Some(shown.text);
        }
        match store::read_capped_in(&self.dir, SHOWN_FILE, SHOWN_CAP as u64) {
            Ok(SourceRead::Ready(bytes)) => std::str::from_utf8(&bytes)
                .ok()
                .and_then(|text| parse_shown(text, chat, message))
                .map(str::to_owned),
            _ => None,
        }
    }

    /// What the board message is. The checkpoint is read first and always: a
    /// file that cannot be read is a hold, and nothing, memory included,
    /// writes over it. Then a message Telegram accepted and the disk refused
    /// takes the place of what the file says, and is written again.
    fn live(&mut self, chat: &str, err: &mut impl Write) -> Result<Live, String> {
        if !valid_chat(chat) {
            return Err("the configured chat id cannot be checkpointed".to_owned());
        }
        let disk = match read_pin(&self.dir) {
            PinRead::Hold(why) => return Err(why),
            PinRead::Found(pin) if pin.chat == chat => Some(pin),
            // No checkpoint, or one for another chat: this chat has no
            // message of ours yet.
            PinRead::Absent | PinRead::Found(_) => None,
        };
        if self
            .unsaved_pin
            .as_ref()
            .is_some_and(|pin| pin.chat != chat)
        {
            self.unsaved_pin = None;
        }
        let pin = if let Some(pin) = self.unsaved_pin.clone() {
            self.save_pin(&pin, err);
            pin
        } else {
            disk.unwrap_or_else(|| Pin {
                chat: chat.to_owned(),
                placed: None,
            })
        };
        let shown = match pin.placed {
            Some(placed) => self.shown_text(chat, placed.message),
            None => None,
        };
        Ok(Live { pin, shown })
    }

    /// One pass: the calls the board needs now, each independent of the
    /// others' failure. A message is sent when there is none, edited when it
    /// does not show the board, and pinned when it is not pinned and its id
    /// is on disk. A message found gone is dropped, and the next pass sends.
    pub(super) fn pass(
        &mut self,
        api: &Api,
        chat: &str,
        desk: Option<&Desk>,
        now: i64,
        err: &mut impl Write,
    ) -> Pass {
        let live = match self.live(chat, err) {
            Ok(live) => live,
            Err(why) => return self.hold(why, err),
        };
        let Some(desk) = desk else {
            return Pass::default();
        };
        let text = render(desk);
        let placed = if let Some(placed) = live.pin.placed {
            let stale = live.shown.as_deref() != Some(text.as_str());
            if stale && self.edit.due(now) && self.edit(api, chat, placed.message, &text, now, err)
            {
                return Pass::default();
            }
            placed
        } else {
            if !self.send.due(now) {
                return Pass::default();
            }
            let Some(message) = self.send(api, chat, &text, now, err) else {
                return Pass::default();
            };
            Placed {
                message,
                pinned: false,
            }
        };
        if !placed.pinned && self.unsaved_pin.is_none() && self.pin.due(now) {
            self.pin(api, chat, placed.message, now, err);
        }
        Pass::default()
    }

    /// Send the board; the new message id when Telegram accepted it.
    fn send(
        &mut self,
        api: &Api,
        chat: &str,
        text: &str,
        now: i64,
        err: &mut impl Write,
    ) -> Option<i64> {
        match api.send_html(text) {
            Ok(message) => {
                self.send.reset();
                self.edit.reset();
                self.pin.reset();
                self.said = None;
                self.save_shown(&Shown {
                    chat: chat.to_owned(),
                    message,
                    text: text.to_owned(),
                });
                self.save_pin(
                    &Pin {
                        chat: chat.to_owned(),
                        placed: Some(Placed {
                            message,
                            pinned: false,
                        }),
                    },
                    err,
                );
                Some(message)
            }
            Err(failure) => {
                self.send.fail(now, asked(failure));
                self.tell(err, format!("the board was not sent: {}", failure.label()));
                None
            }
        }
    }

    /// Edit `message` to show the board; `true` when Telegram says it is gone,
    /// which clears the checkpoint's message so the next pass sends a new one.
    fn edit(
        &mut self,
        api: &Api,
        chat: &str,
        message: i64,
        text: &str,
        now: i64,
        err: &mut impl Write,
    ) -> bool {
        match api.edit_html(message, text) {
            Ok(_) => {
                self.edit.reset();
                self.said = None;
                self.save_shown(&Shown {
                    chat: chat.to_owned(),
                    message,
                    text: text.to_owned(),
                });
                false
            }
            Err(DeskFailure::Gone) => {
                self.edit.reset();
                self.pin.reset();
                self.said = None;
                self.tell(
                    err,
                    format!("board message {message} is gone; the next pass sends a new one"),
                );
                self.save_pin(
                    &Pin {
                        chat: chat.to_owned(),
                        placed: None,
                    },
                    err,
                );
                true
            }
            Err(failure) => {
                self.edit.fail(now, asked(failure));
                self.tell(
                    err,
                    format!("the board was not edited: {}", failure.label()),
                );
                false
            }
        }
    }

    fn pin(&mut self, api: &Api, chat: &str, message: i64, now: i64, err: &mut impl Write) {
        match api.pin(message) {
            Ok(()) => {
                self.pin.reset();
                self.said = None;
                self.save_pin(
                    &Pin {
                        chat: chat.to_owned(),
                        placed: Some(Placed {
                            message,
                            pinned: true,
                        }),
                    },
                    err,
                );
            }
            Err(failure) => {
                self.pin.fail(now, asked(failure));
                self.tell(
                    err,
                    format!("the board was not pinned: {}", failure.label()),
                );
            }
        }
    }
}
