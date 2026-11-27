//! The SGR reading of a captured pane: its styled runs, and the plain rows
//! `capture-pane -p` would print from the same capture.
//!
//! Pure: it takes a capture and answers with its runs, and runs no tmux. The
//! input-region sensor (`deliver::region`) and the dictation classifier
//! (`harness_state`) read styled captures through this one parser.

/// The foreground a captured run is painted in, when the capture names one.
/// Only IDENTITY matters: a run is compared against the colour the box paints
/// its OWN rule in, so the frame carries its own reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fg {
    /// An SGR 30-37 / 90-97 named colour.
    Named(u8),
    /// An `38;5;N` indexed colour.
    Indexed(u8),
    /// An `38;2;R;G;B` truecolour.
    Rgb(u8, u8, u8),
}

/// One run of captured text sharing an SGR intensity state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// SGR 1 is active.
    pub bold: bool,
    /// SGR 2 is active.
    pub dim: bool,
    /// SGR 7 is active: claude paints its cursor cell this way, over the
    /// first cell of a placeholder.
    pub inverse: bool,
    /// The foreground, when the frame names one.
    pub fg: Option<Fg>,
    /// The printable run.
    pub text: String,
    /// Its row in the region, counting from 0.
    pub line: usize,
}

/// Parse a captured region into styled segments — `_sgr_parse`.
#[must_use]
pub fn parse(region: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut rest = region;
    let (mut bold, mut dim, mut inverse, mut line) = (false, false, false, 0usize);
    let mut fg: Option<Fg> = None;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("\u{1b}]") {
            rest = consume_osc(after, &mut line);
            continue;
        }
        if let Some(body) = rest.strip_prefix("\u{1b}[") {
            let split = body.char_indices().find(|(_, ch)| ch.is_ascii_alphabetic());
            let Some((at, final_byte)) = split else {
                // No final byte at all: consume the whole remainder rather
                // than spinning on it.
                break;
            };
            let params = &body[..at];
            rest = &body[at + final_byte.len_utf8()..];
            if final_byte != 'm' {
                continue; // a non-SGR CSI: consumed, no state change
            }
            apply_sgr(
                if params.is_empty() { "0" } else { params },
                &mut bold,
                &mut dim,
                &mut inverse,
                &mut fg,
            );
            continue;
        }
        if rest.starts_with('\u{1b}') {
            // A non-CSI escape (a charset designator, `\e7`, …). tmux -e emits
            // SGR only, so this is defensive — but it MUST consume bytes: the
            // frozen loop once matched an empty text run here and spun forever.
            rest = consume_escape(&rest[1..]);
            continue;
        }
        let text = rest.split('\u{1b}').next().unwrap_or_default();
        rest = &rest[text.len()..];
        if text.is_empty() {
            continue;
        }
        // Split the run at newlines so every segment belongs to exactly ONE row.
        let mut parts = text.split('\n');
        let mut chunk = parts.next().unwrap_or_default();
        loop {
            if !chunk.is_empty() {
                segments.push(Segment {
                    bold,
                    dim,
                    inverse,
                    fg,
                    text: chunk.to_owned(),
                    line,
                });
            }
            match parts.next() {
                Some(next) => {
                    line += 1;
                    chunk = next;
                }
                None => break,
            }
        }
    }
    segments
}

/// The printable rows of a captured region, escapes dropped and trailing
/// blanks trimmed — what `capture-pane -p` prints, read off the SAME capture.
#[must_use]
pub(crate) fn plain_text(region: &str) -> String {
    plain_rows(&parse(region))
}

pub(crate) fn plain_rows(segments: &[Segment]) -> String {
    let mut rows: Vec<String> = Vec::new();
    for segment in segments {
        if rows.len() <= segment.line {
            rows.resize(segment.line + 1, String::new());
        }
        rows[segment.line].push_str(&segment.text);
    }
    let rows: Vec<&str> = rows.iter().map(|row| row.trim_end_matches(' ')).collect();
    rows.join("\n")
}

/// Consume an OSC through BEL or ST, returning what follows.
fn consume_osc<'a>(mut rest: &'a str, line: &mut usize) -> &'a str {
    while let Some(ch) = rest.chars().next() {
        if ch == '\u{7}' {
            return &rest[1..];
        }
        if let Some(after) = rest.strip_prefix("\u{1b}\\") {
            return after;
        }
        if ch == '\n' {
            *line += 1;
            return &rest[1..];
        }
        rest = &rest[ch.len_utf8()..];
    }
    rest
}

/// Consume a non-CSI escape's intermediates and its one final byte.
fn consume_escape(mut rest: &str) -> &str {
    while let Some(ch) = rest.chars().next() {
        let intermediate = (' '..='/').contains(&ch);
        rest = &rest[ch.len_utf8()..];
        if !intermediate {
            break;
        }
    }
    rest
}

/// Apply one SGR parameter list to the intensity, inverse and foreground state.
fn apply_sgr(
    params: &str,
    bold: &mut bool,
    dim: &mut bool,
    inverse: &mut bool,
    fg: &mut Option<Fg>,
) {
    let list: Vec<&str> = params.split(';').collect();
    let mut index = 0;
    while index < list.len() {
        let value = list[index];
        match value {
            // 0 and its empty spelling reset the inverse and the foreground
            // too; 21 and 22 turn intensity off only, so both land in one arm
            // here.
            "" | "0" => {
                *bold = false;
                *dim = false;
                *inverse = false;
                *fg = None;
            }
            "21" | "22" => {
                *bold = false;
                *dim = false;
            }
            "1" => *bold = true,
            "2" => *dim = true,
            "7" => *inverse = true,
            "27" => *inverse = false,
            "38" => match list.get(index + 1).copied() {
                // 38;5;N or 38;2;R;G;B: the colour is the run's identity.
                Some("5") => {
                    *fg = list
                        .get(index + 2)
                        .and_then(|n| n.parse::<u8>().ok())
                        .map(Fg::Indexed);
                    index += 2;
                }
                Some("2") => {
                    let channels = [
                        list.get(index + 2),
                        list.get(index + 3),
                        list.get(index + 4),
                    ]
                    .map(|channel| channel.and_then(|n| n.parse::<u8>().ok()));
                    *fg = match channels {
                        [Some(red), Some(green), Some(blue)] => Some(Fg::Rgb(red, green, blue)),
                        _ => None,
                    };
                    index += 4;
                }
                _ => {}
            },
            // The background and every other attribute are consumed, never
            // tracked: this sensor reads the identity of a run, not its look.
            "48" => match list.get(index + 1).copied() {
                Some("5") => index += 2,
                Some("2") => index += 4,
                _ => {}
            },
            _ => {
                if let Ok(code) = value.parse::<u8>() {
                    match code {
                        30..=37 | 90..=97 => *fg = Some(Fg::Named(code)),
                        39 => *fg = None,
                        _ => {}
                    }
                }
            }
        }
        index += 1;
    }
}
