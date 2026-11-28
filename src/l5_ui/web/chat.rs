//! The orchestrator chat as the page shows it: the app panel's lane, each
//! turn with the app's own header words. The state render is the outbound
//! boundary: it keeps the newest [`ROWS_MAX`] rows and makes every text plain,
//! masked and capped with [`plain`] and [`words`], whatever built the chat.

use std::borrow::Cow;
use std::collections::BTreeSet;

use crate::app::{LaneVoice, lane_head, lane_key};
use crate::console::lane::{Kind, Lane};
use crate::topic;

/// The most rows a snapshot carries, newest kept.
pub const ROWS_MAX: usize = 500;
/// The most bytes of one row's body or one coverage line.
pub const BODY_MAX: usize = 65_536;
/// The most characters of a speaker, a qualifier or a name.
pub const WORDS_MAX: usize = 200;
/// How many bytes of a row's body [`row_tags`] reads for `#slug` tags.
pub const TAGS_WINDOW: usize = 512;
/// What a served text shows in place of an `ae web` one-time code.
pub const CODE_MASK: &str = "#c=\u{2026}";
/// The fewest hex digits after `#c=` that [`mask`] reads as a code: a minted
/// code is 32 (`auth::is_hex`).
const CODE_HEX: usize = 32;

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

/// The topics of `known` that `body` tags, in text order, each once. Only the
/// first [`TAGS_WINDOW`] bytes count, which bounds the scan for every row of a
/// snapshot; a tag the window would cut short is not read.
#[must_use]
pub fn row_tags<'a>(body: &'a str, known: &BTreeSet<&str>) -> Vec<&'a str> {
    let mut tags = topic::tags(window(body));
    tags.retain(|tag| known.contains(tag));
    tags
}

/// The part of `body` the tag scan reads. When the character after the window
/// cannot continue a slug, it is read too, so a tag that ends at the edge is
/// judged by the character that follows it, as in the whole text. Otherwise
/// the cut falls before the last whitespace in the window, so no part of a
/// longer word reads as a tag.
fn window(body: &str) -> &str {
    if body.len() <= TAGS_WINDOW {
        return body;
    }
    let mut end = TAGS_WINDOW;
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    let (head, tail) = body.split_at(end);
    match tail.chars().next() {
        Some(next) if !matches!(next, 'a'..='z' | '0'..='9' | '-') => {
            body.get(..end + next.len_utf8()).unwrap_or(head)
        }
        _ => head
            .rfind(char::is_whitespace)
            .and_then(|cut| head.get(..cut))
            .unwrap_or_default(),
    }
}

/// `text` with every control character but newline and tab removed, then
/// every one-time code masked ([`mask`]), then cut at a character boundary to
/// at most `cap` bytes; borrowed when nothing changes, which a byte scan
/// decides (`0xC2` leads every C1 control and `#c=` may start a code, so either
/// sends a text to the careful path). The removal comes first, so a control
/// character inside a code cannot hide it, and the cut comes last, so it never
/// shows a code's first digits.
#[must_use]
pub fn plain(text: &str, cap: usize) -> Cow<'_, str> {
    if text.len() <= cap && clean(text.as_bytes()) {
        return Cow::Borrowed(text);
    }
    let kept: String = text
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .collect();
    let masked = mask(&kept);
    let mut end = masked.len().min(cap);
    while !masked.is_char_boundary(end) {
        end -= 1;
    }
    Cow::Owned(masked.get(..end).unwrap_or_default().to_owned())
}

/// Whether `bytes` holds no control byte but newline and tab, no `0xC2` and no
/// `#c=`. An indexed loop, not an iterator chain: an unoptimized build scans
/// the 32 MiB of a full chat in a fraction of the first-byte budget this way.
fn clean(bytes: &[u8]) -> bool {
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        if (byte < 0x20 && byte != b'\n' && byte != b'\t') || byte == 0x7f || byte == 0xc2 {
            return false;
        }
        if byte == b'#' && bytes.get(at + 1) == Some(&b'c') && bytes.get(at + 2) == Some(&b'=') {
            return false;
        }
        at += 1;
    }
    true
}

/// `text` as one line of words: every control character removed, every
/// one-time code masked ([`mask`]), then cut to at most [`WORDS_MAX`]
/// characters.
#[must_use]
pub fn words(text: &str) -> String {
    let kept: String = text.chars().filter(|ch| !ch.is_control()).collect();
    mask(&kept).chars().take(WORDS_MAX).collect()
}

/// `text` with each `#c=` that a run of at least [`CODE_HEX`] ASCII hex digits
/// follows written as [`CODE_MASK`], the whole run replaced; borrowed when no
/// code is found. A shorter run stays as written.
#[must_use]
pub fn mask(text: &str) -> Cow<'_, str> {
    let mut out = String::new();
    let (mut copied, mut at) = (0, 0);
    while let Some(found) = text.get(at..).and_then(|rest| rest.find("#c=")) {
        let digits = at + found + 3;
        let run = text
            .as_bytes()
            .get(digits..)
            .unwrap_or_default()
            .iter()
            .take_while(|byte| byte.is_ascii_hexdigit())
            .count();
        if run >= CODE_HEX {
            out.push_str(text.get(copied..at + found).unwrap_or_default());
            out.push_str(CODE_MASK);
            copied = digits + run;
            at = copied;
        } else {
            at = digits;
        }
    }
    if copied == 0 {
        return Cow::Borrowed(text);
    }
    out.push_str(text.get(copied..).unwrap_or_default());
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::collections::BTreeSet;

    use super::{TAGS_WINDOW, plain, row_tags};

    /// A body of exactly [`TAGS_WINDOW`] bytes is read whole, and a longer one
    /// keeps a tag whose closer is the first character past the window, even
    /// when that character is more than one byte.
    #[test]
    fn a_tag_at_the_window_edge_is_read_whole() {
        let known = BTreeSet::from(["calm"]);
        let whole = format!("{} #calm", "x".repeat(TAGS_WINDOW - 6));
        assert_eq!(whole.len(), TAGS_WINDOW);
        assert_eq!(row_tags(&whole, &known), ["calm"]);
        assert_eq!(row_tags(&format!("{whole}\u{3000}end"), &known), ["calm"]);
        assert!(row_tags(&format!("{whole}9 end"), &known).is_empty());
    }

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
