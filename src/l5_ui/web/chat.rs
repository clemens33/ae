//! The orchestrator chat as the page shows it: the app panel's lane, each
//! turn with the app's own header words. The state render is the outbound
//! boundary: it keeps the newest [`ROWS_MAX`] rows and makes every text plain
//! and capped with [`plain`] and [`words`], whatever built the chat.

use std::borrow::Cow;

use crate::app::{LaneVoice, lane_head, lane_key};
use crate::console::lane::{Kind, Lane};

/// The most rows a snapshot carries, newest kept.
pub const ROWS_MAX: usize = 500;
/// The most bytes of one row's body or one coverage line.
pub const BODY_MAX: usize = 65_536;
/// The most characters of a speaker, a qualifier or a name.
pub const WORDS_MAX: usize = 200;

/// One turn of the chat.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Row {
    /// The app's turn key: its identity across reads.
    pub key: u64,
    /// When, in epoch microseconds.
    pub micros: i64,
    /// What the turn is: the lane kind's name in snake case.
    pub kind: String,
    /// The speaker, as the app's header names it.
    pub who: String,
    /// What the turn is, as the app's header says it.
    pub what: String,
    /// `you`, `lead`, `colead` or `system`.
    pub voice: String,
    /// The turn's text.
    pub body: String,
}

/// The chat: its rows oldest first, its coverage gaps, and how many older
/// rows were left out.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Chat {
    /// The rows, oldest first.
    pub rows: Vec<Row>,
    /// One line per gap in what was read.
    pub coverage: Vec<String>,
    /// The older rows left out.
    pub cut: usize,
}

/// The page's chat for `lane`, whose lead pair's main seat is `main`: the
/// newest [`ROWS_MAX`] turns with the app's header words, text unchanged.
#[must_use]
pub fn rows(lane: &Lane, main: &str) -> Chat {
    let cut = lane.items.len().saturating_sub(ROWS_MAX);
    let rows = lane
        .items
        .iter()
        .skip(cut)
        .map(|item| {
            let (who, what, voice) = lane_head(&item.kind, main, false);
            Row {
                key: lane_key(item),
                micros: item.micros,
                kind: kind_word(&item.kind).to_owned(),
                who,
                what,
                voice: voice_word(voice).to_owned(),
                body: item.body.clone(),
            }
        })
        .collect();
    Chat {
        rows,
        coverage: lane.coverage.clone(),
        cut,
    }
}

/// The name of `kind`, snake case.
fn kind_word(kind: &Kind) -> &'static str {
    match kind {
        Kind::Pane { .. } => "pane",
        Kind::Assistant { .. } => "assistant",
        Kind::Inbound { .. } => "inbound",
        Kind::Reply { .. } => "reply",
        Kind::Said { .. } => "said",
        Kind::Card { .. } => "card",
        Kind::PastCard { .. } => "past_card",
        Kind::NeedsYou { .. } => "needs_you",
        Kind::Asked { .. } => "asked",
        Kind::NotDelivered { .. } => "not_delivered",
        Kind::Closed { .. } => "closed",
        Kind::Answer { .. } => "answer",
        Kind::Unadmitted { .. } => "unadmitted",
        Kind::Accepted { .. } => "accepted",
        Kind::Fleet { .. } => "fleet",
    }
}

/// The word of `voice`.
fn voice_word(voice: LaneVoice) -> &'static str {
    match voice {
        LaneVoice::You => "you",
        LaneVoice::Lead => "lead",
        LaneVoice::Colead => "colead",
        LaneVoice::System => "system",
    }
}

/// `text` with every control character but newline and tab removed, then
/// cut at a character boundary to at most `cap` bytes; borrowed when nothing
/// changes, which a byte scan decides (`0xC2` leads every C1 control, so it
/// sends a text to the careful path).
#[must_use]
pub fn plain(text: &str, cap: usize) -> Cow<'_, str> {
    if text.len() <= cap && clean(text.as_bytes()) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len().min(cap));
    for ch in text
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
    {
        if out.len() + ch.len_utf8() > cap {
            break;
        }
        out.push(ch);
    }
    Cow::Owned(out)
}

/// Whether `bytes` holds no control byte but newline and tab, and no `0xC2`.
/// An indexed loop, not an iterator chain: an unoptimized build scans the
/// 32 MiB of a full chat in a fraction of the first-byte budget this way.
fn clean(bytes: &[u8]) -> bool {
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        if (byte < 0x20 && byte != b'\n' && byte != b'\t') || byte == 0x7f || byte == 0xc2 {
            return false;
        }
        at += 1;
    }
    true
}

/// `text` as one line of words: every control character removed, then cut to
/// at most [`WORDS_MAX`] characters.
#[must_use]
pub fn words(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_control())
        .take(WORDS_MAX)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::plain;

    /// docs/web.md: every agent text is plain, control characters removed, and
    /// capped. A short text with a control byte is cleaned, not passed through,
    /// and a clean text over the cap is cut, not passed through.
    #[test]
    fn plain_cleans_a_short_text_and_cuts_a_clean_long_one() {
        assert_eq!(plain("a\u{7}b\tc\n", 64), "ab\tc\n");
        assert_eq!(plain("abcdef", 3), "abc");
    }

    /// docs/web.md and `plain`: a text with no control character but newline
    /// and tab is borrowed unchanged, and every other control character is
    /// removed: C0 without a newline or tab, DEL, and C1 (U+0085 is C2 85).
    #[test]
    fn plain_borrows_a_clean_text_and_removes_every_control_character() {
        let clean = "a plain\ttext\nline";
        assert!(
            matches!(plain(clean, 64), Cow::Borrowed(text) if text == clean),
            "a clean text is borrowed unchanged"
        );
        assert_eq!(plain("a\u{7}b", 64), "ab");
        assert_eq!(plain("a\u{7f}b\u{85}c", 64), "abc");
    }
}
