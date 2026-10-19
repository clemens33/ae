//! A decision card's reason, laid out for reading.
//!
//! An agent declares `waiting-user` with one line of prose. [`layout`] cuts it
//! into the lines a human reads: the question, one line per option, the rest
//! sentence by sentence, and a trailing evidence pointer last, with one gap
//! between those blocks. Every piece is a slice of the reason, in its order:
//! nothing is dropped, moved or rewritten, and a renderer adds only its margin,
//! a bullet and the gaps. The reason is hostile persisted text (the journal),
//! so the cut is total: any text lays out, and none panics.

/// How a [`Part`] is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A line of the question, or of the prose after the options.
    Text,
    /// One option. `label` counts the leading bytes of the text that are its
    /// own label (`1 `, `a) `); zero means it wears a bullet. Its wrapped rows
    /// hang under the text after the label or the bullet.
    Option { label: usize },
    /// The trailing evidence pointer, dim where the look is drawn.
    Evidence,
    /// The empty row between two blocks. Its text is empty.
    Gap,
}

/// One line of a laid-out reason: a slice of it, and how it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part<'a> {
    pub role: Role,
    pub text: &'a str,
}

/// Words that end in a period without ending a sentence.
const ABBREVIATIONS: [&str; 6] = ["e.g.", "i.e.", "etc.", "vs.", "cf.", "approx."];

/// The spacing a cut may spend: whitespace the terminal renderer keeps as
/// whitespace. A control character it turns into a visible replacement is a
/// word of its own and stays.
fn is_gap(ch: char) -> bool {
    ch.is_whitespace() && (ch == '\t' || ch == '\n' || !ch.is_control())
}

fn trim(text: &str) -> &str {
    text.trim_matches(is_gap)
}

/// The lines `reason` reads as. A reason with a standalone `|` is SHAPED: a
/// question, then the options the pipes separate, then the prose after the
/// last option. Any other reason is prose alone.
#[must_use]
pub fn layout(reason: &str) -> Vec<Part<'_>> {
    let pieces = pipes(reason);
    let (question, options, tail) = if pieces.len() > 1 {
        shaped(&pieces)
    } else {
        ("", Vec::new(), trim(reason))
    };
    let labelled = options.len() > 1 && family(options[1]).is_some();
    let mut blocks: Vec<Vec<Part<'_>>> = Vec::new();
    blocks.push(question_lines(question));
    blocks.push(
        options
            .into_iter()
            .map(|text| Part {
                role: Role::Option {
                    label: if labelled { label_len(text) } else { 0 },
                },
                text,
            })
            .collect(),
    );
    let (prose, evidence) = tail_lines(tail);
    blocks.push(prose);
    blocks.push(evidence);
    let mut parts = Vec::new();
    for block in blocks.into_iter().filter(|block| !block.is_empty()) {
        if !parts.is_empty() {
            parts.push(Part {
                role: Role::Gap,
                text: "",
            });
        }
        parts.extend(block);
    }
    parts
}

/// `reason` cut at every standalone `|`: one with a gap or an end on both sides.
fn pipes(reason: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut from = 0;
    let mut before: Option<char> = None;
    let mut chars = reason.char_indices().peekable();
    while let Some((at, ch)) = chars.next() {
        let after = chars.peek().map(|&(_, next)| next);
        if ch == '|' && before.is_none_or(is_gap) && after.is_none_or(is_gap) {
            pieces.push(&reason[from..at]);
            from = at + 1;
        }
        before = Some(ch);
    }
    pieces.push(&reason[from..]);
    pieces
}

/// The question, the options and the prose after them, of a reason the pipes
/// cut into `pieces`; a wordless piece is no option.
fn shaped<'a>(pieces: &[&'a str]) -> (&'a str, Vec<&'a str>, &'a str) {
    let mut pieces = pieces
        .iter()
        .map(|piece| trim(piece))
        .filter(|piece| !piece.is_empty());
    let Some(first) = pieces.next() else {
        return ("", Vec::new(), "");
    };
    let rest: Vec<&str> = pieces.collect();
    let start = option_start(first, rest.first().copied());
    let question = trim(&first[..start]);
    let mut options = vec![trim(&first[start..])];
    options.extend(rest);
    let mut tail = "";
    if let Some(last) = options.last_mut() {
        let cut = option_end(last);
        tail = trim(&last[cut..]);
        *last = trim(&last[..cut]);
    }
    options.retain(|option| !option.is_empty());
    (question, options, tail)
}

/// Where the first option starts inside the first piece: at its own label when
/// the second option opens with the next one (`1 `…`2 `), else after the
/// piece's last sentence, else after its first `: `, else at its start.
fn option_start(first: &str, second: Option<&str>) -> usize {
    if let Some(label) = second.and_then(family) {
        let found = first.match_indices(label).find_map(|(at, _)| {
            (at == 0 || first[..at].ends_with(". ") || first[..at].ends_with(": ")).then_some(at)
        });
        if let Some(at) = found {
            return at;
        }
    }
    if let Some(&(_, start)) = boundaries(first).last() {
        return start;
    }
    first.find(": ").map_or(0, |at| {
        let rest = &first[at + 1..];
        at + 1 + (rest.len() - rest.trim_start_matches(is_gap).len())
    })
}

/// The label the option BEFORE `next` wears, when `next` opens with the
/// second of a family: `2 ` asks for `1 `, `b)` for `a)`.
fn family(next: &str) -> Option<&'static str> {
    match next.get(..2)? {
        "2 " => Some("1 "),
        "2." => Some("1."),
        "2)" => Some("1)"),
        "b " => Some("a "),
        "b." => Some("a."),
        "b)" => Some("a)"),
        _ => None,
    }
}

/// The bytes of `option`'s own label and the one space after it, when it
/// opens with one: an ASCII letter or digit, then a space, or a `.` or `)` and
/// at most one space. Any further spaces stay the words' own indent, so a mark
/// is never wider than three cells.
fn label_len(option: &str) -> usize {
    match option.as_bytes() {
        [first, b' ', ..] if first.is_ascii_alphanumeric() => 2,
        [first, b'.' | b')', b' ', ..] if first.is_ascii_alphanumeric() => 3,
        [first, b'.' | b')', ..] if first.is_ascii_alphanumeric() => 2,
        _ => 0,
    }
}

/// Where the last option ends: after its first sentence, or before a
/// `(recommend …)` that follows it, whichever comes first.
fn option_end(last: &str) -> usize {
    let sentence = boundaries(last).first().map_or(last.len(), |&(end, _)| end);
    let recommend = [" (recommend ", " (Recommend "]
        .iter()
        .filter_map(|marker| last.find(marker))
        .min()
        .map_or(last.len(), |at| at);
    sentence.min(recommend)
}

/// Each sentence end of `text` as (the end of the sentence, the start of the
/// next): `.`, `?` or `!` and then a space, outside parentheses, never the dot
/// of an abbreviation, of an ellipsis or of an option's own `1.` label, and
/// never at the very end.
fn boundaries(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut previous = None;
    let mut chars = text.char_indices().peekable();
    while let Some((at, ch)) = chars.next() {
        let next = chars.peek().map(|&(_, next)| next);
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            '.' | '?' | '!' if depth == 0 && next == Some(' ') => {
                let end = at + ch.len_utf8();
                let dot = ch == '.';
                let label =
                    dot && at == 1 && previous.is_some_and(|ch: char| ch.is_ascii_alphanumeric());
                if !(dot && (label || previous == Some('.') || abbreviation(&text[..end]))) {
                    let rest = &text[end..];
                    let start = end + (rest.len() - rest.trim_start_matches(is_gap).len());
                    if start < text.len() {
                        out.push((end, start));
                    }
                }
            }
            _ => {}
        }
        previous = Some(ch);
    }
    out
}

/// Whether `head` ends with one of the [`ABBREVIATIONS`].
fn abbreviation(head: &str) -> bool {
    let word = head.rsplit(is_gap).next().unwrap_or(head);
    let word = word.trim_start_matches(|ch: char| !ch.is_alphanumeric());
    ABBREVIATIONS
        .iter()
        .any(|known| word.eq_ignore_ascii_case(known))
}

/// `text` sentence by sentence, each trimmed, none empty.
fn sentences(text: &str) -> Vec<&str> {
    spans(text)
        .into_iter()
        .map(|(from, to)| &text[from..to])
        .collect()
}

/// Where each of [`sentences`] lies in `text`, as byte ranges.
fn spans(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    let mut cut = |from: usize, to: usize| {
        let piece = &text[from..to];
        let lead = piece.len() - piece.trim_start_matches(is_gap).len();
        let body = trim(piece);
        if !body.is_empty() {
            out.push((from + lead, from + lead + body.len()));
        }
    };
    for (end, start) in boundaries(text) {
        cut(from, end);
        from = start;
    }
    cut(from, text.len());
    out
}

/// The question's lines: its head up to the first `: `, then the rest, each
/// sentence on its own line.
fn question_lines(question: &str) -> Vec<Part<'_>> {
    let (head, rest) = match question.find(": ") {
        Some(at) => (&question[..=at], &question[at + 1..]),
        None => (question, ""),
    };
    let text = |text| Part {
        role: Role::Text,
        text,
    };
    let mut lines: Vec<Part<'_>> = sentences(head).into_iter().map(text).collect();
    lines.extend(sentences(rest).into_iter().map(text));
    lines
}

/// The prose after the options, one line per sentence, and its trailing run
/// of evidence pointers as one last line.
fn tail_lines(tail: &str) -> (Vec<Part<'_>>, Vec<Part<'_>>) {
    let all = spans(tail);
    let pointers = all
        .iter()
        .rev()
        .take_while(|&&(from, to)| evidence(&tail[from..to]))
        .count();
    let keep = all.len() - pointers;
    let prose = all[..keep]
        .iter()
        .map(|&(from, to)| Part {
            role: Role::Text,
            text: &tail[from..to],
        })
        .collect();
    let evidence = all.get(keep).map_or_else(Vec::new, |&(from, _)| {
        vec![Part {
            role: Role::Evidence,
            text: &tail[from..],
        }]
    });
    (prose, evidence)
}

/// Whether `sentence` is a pointer to evidence rather than a claim: it names a
/// `.local/` path, `details:` or `Evidence:`, opens with `Details` or a
/// `memo <topic>`, or is a bare `file:line` reference. A recommendation never is.
fn evidence(sentence: &str) -> bool {
    if recommends(sentence) {
        return false;
    }
    let opens = |word: &str| {
        sentence
            .get(..word.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(word))
    };
    sentence.contains(".local/")
        || sentence.contains("details:")
        || sentence.contains("Details:")
        || sentence.contains("Evidence:")
        || sentence.contains("evidence:")
        || opens("details ")
        || opens("memo ")
        || reference(sentence)
}

/// Whether `sentence` opens with a recommendation.
fn recommends(sentence: &str) -> bool {
    let word = sentence.strip_prefix('(').unwrap_or(sentence);
    word.get(..9)
        .is_some_and(|head| head.eq_ignore_ascii_case("recommend"))
}

/// Whether `sentence` is only a `path:line` (`src/x.rs:12.`, `see src/x.rs:12`).
fn reference(sentence: &str) -> bool {
    let token = sentence.trim_end_matches('.');
    let token = ["see ", "See "]
        .iter()
        .find_map(|see| token.strip_prefix(see))
        .unwrap_or(token);
    if token.contains(is_gap) {
        return false;
    }
    token.rsplit_once(':').is_some_and(|(path, line)| {
        !line.is_empty()
            && line.bytes().all(|byte| byte.is_ascii_digit())
            && (path.contains('.') || path.contains('/'))
    })
}

#[cfg(test)]
mod tests {
    use super::{Part, Role, layout};

    fn texts<'a>(parts: &[Part<'a>]) -> Vec<(Role, &'a str)> {
        parts.iter().map(|part| (part.role, part.text)).collect()
    }

    #[test]
    fn every_part_is_a_slice_of_the_reason_in_its_order() {
        for reason in [
            "Q? a | b. Recommend a. Memo x.",
            "A | | B",
            "| B",
            "",
            "(recommend A. B forever",
            "word\u{a0}| other",
        ] {
            let parts = layout(reason);
            let mut at = 0;
            for part in parts.iter().filter(|part| part.role != Role::Gap) {
                let offset = part.text.as_ptr() as usize - reason.as_ptr() as usize;
                assert!(offset >= at, "{reason:?}");
                at = offset + part.text.len();
                assert!(!part.text.is_empty(), "{reason:?}");
            }
        }
    }

    #[test]
    fn a_labelled_option_counts_its_label_and_a_bulleted_one_none() {
        assert_eq!(
            texts(&layout("Steps. 1. merge | 2. wait")),
            [
                (Role::Text, "Steps."),
                (Role::Gap, ""),
                (Role::Option { label: 3 }, "1. merge"),
                (Role::Option { label: 3 }, "2. wait"),
            ]
        );
        assert_eq!(
            texts(&layout("a | b")),
            [
                (Role::Option { label: 0 }, "a"),
                (Role::Option { label: 0 }, "b"),
            ]
        );
    }
}
