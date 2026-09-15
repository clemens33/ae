//! The input-region sensor: what is in a TUI's input box right now.
//!
//! Ported from `ae`'s `_sgr_parse`, `_input_region_prompt_seg`,
//! `_input_region_content_end`, `_input_region_is_border` and
//! `_input_region_occupied` — the pane-reading half of the delivery path, and
//! the half where every shipped delivery bug lived. Nothing here runs tmux:
//! it takes a capture and answers a question about it, so the whole model is
//! unit-testable against recorded frames.

use crate::tool::InputModel;

/// One run of captured text sharing an SGR intensity state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// SGR 1 is active.
    pub bold: bool,
    /// SGR 2 is active.
    pub dim: bool,
    /// The printable run.
    pub text: String,
    /// Its row in the region, counting from 0.
    pub line: usize,
}

/// What the sensor read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Occupancy {
    /// Real typed or staged content sits in the input box.
    Occupied,
    /// A bare ornament, or a dim placeholder suggestion only.
    Idle,
    /// No live prompt in view, or nothing to read.
    Unreadable,
}

/// Where the input box ENDS, and how that row is recognised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopAt {
    /// claude: the box's bottom border — a row whose printable content is
    /// ENTIRELY U+2500 and which spans the full width of the region.
    Border,
    /// codex: no bottom border at all, so the LAST blank row below the prompt
    /// (its separator above the model/path footer).
    Blank,
}

/// Parse a captured region into styled segments — `_sgr_parse`.
#[must_use]
pub fn parse(region: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut rest = region;
    let (mut bold, mut dim, mut line) = (false, false, 0usize);
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

/// Apply one SGR parameter list to the intensity state.
fn apply_sgr(params: &str, bold: &mut bool, dim: &mut bool) {
    let list: Vec<&str> = params.split(';').collect();
    let mut index = 0;
    while index < list.len() {
        let value = list[index];
        match value {
            // 0 and its empty spelling reset both; 21 and 22 turn each off,
            // and ae only tracks intensity, so all four land in one arm.
            "" | "0" | "21" | "22" => {
                *bold = false;
                *dim = false;
            }
            "1" => *bold = true,
            "2" => *dim = true,
            "38" | "48" => match list.get(index + 1).copied() {
                // 38;5;N (256) or 38;2;R;G;B (truecolour): skip the arguments.
                Some("5") => index += 2,
                Some("2") => index += 4,
                _ => {}
            },
            _ => {}
        }
        index += 1;
    }
}

/// The live prompt found in a parsed region.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Prompt {
    /// Index into the segments.
    index: usize,
    /// That segment's text from the ornament onward.
    tail: String,
    /// Which ornament matched, so a multi-ornament tool strips its own anchor.
    ornament: &'static str,
}

/// The composer row — the BOTTOM-MOST row whose first non-blank run starts with
/// one of `ornaments`.
///
/// Direction is the whole rule: a transcript echo of an already-submitted turn
/// sits ABOVE the live box on every captured frame, so the live composer is
/// always the lowest ornament row. `prompt` (the occupancy read) and
/// `notice::reconstruct` (the notice proof) both select through this one rule,
/// so selection can no longer drift between two loops. `live_only` stays a
/// per-caller strictness choice — the occupancy read asks for its styling
/// filter, the notice proof does not (see `reconstruct`'s comment) — so on a
/// frame where a styled echo sits above a differently-styled composer they may
/// name different ROWS, by that documented choice and never by disagreement
/// about the rule. Returns the row's line index and the matched ornament.
pub(super) fn composer_row(
    segments: &[Segment],
    live_only: bool,
    ornaments: &[&'static str],
) -> Option<(usize, &'static str)> {
    let last = segments.last()?;
    for line in (0..=last.line).rev() {
        let Some(index) = segments
            .iter()
            .position(|seg| seg.line == line && !is_blank(&seg.text))
        else {
            continue;
        };
        let tail = segments[index].text.trim_start_matches(is_space);
        for ornament in ornaments {
            if !tail.starts_with(ornament) {
                continue;
            }
            if live_only && (!segments[index].bold || segments[index].dim) {
                continue;
            }
            return Some((line, *ornament));
        }
    }
    None
}

/// Find the live input prompt — `_input_region_prompt_seg`.
fn prompt(segments: &[Segment], live_only: bool, ornaments: &[&'static str]) -> Option<Prompt> {
    let (line, ornament) = composer_row(segments, live_only, ornaments)?;
    let index = segments
        .iter()
        .position(|seg| seg.line == line && !is_blank(&seg.text))?;
    Some(Prompt {
        index,
        tail: segments[index].text.trim_start_matches(is_space).to_owned(),
        ornament,
    })
}

/// Is this row the input box's structural BORDER — `_input_region_is_border`?
fn is_border(row: &str, max_width: usize) -> bool {
    !row.is_empty() && row.chars().all(|ch| ch == '─') && row.chars().count() == max_width
}

/// Where the input box ends — `_input_region_content_end`.
fn content_end(segments: &[Segment], prompt_line: usize, stop_at: StopAt) -> usize {
    let Some(last) = segments.last() else {
        return 0;
    };
    let mut end = last.line + 1;
    // A border must be judged on the WHOLE row, never on its first cell.
    let mut rows: Vec<String> = vec![String::new(); end];
    for seg in segments {
        if let Some(row) = rows.get_mut(seg.line) {
            row.push_str(&seg.text);
        }
    }
    let max_width = segments
        .iter()
        .map(|seg| seg.line)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter_map(|line| rows.get(line))
        .map(|row| row.chars().count())
        .max()
        .unwrap_or(0);
    let mut nonblank = vec![false; end];
    for seg in segments {
        if seg.line <= prompt_line || seg.line >= end {
            continue;
        }
        if is_blank(&seg.text) {
            continue;
        }
        if let Some(flag) = nonblank.get_mut(seg.line) {
            *flag = true;
        }
        if stop_at == StopAt::Border && is_border(&rows[seg.line], max_width) {
            end = seg.line;
        }
    }
    if stop_at == StopAt::Blank {
        // The LAST blank row below the prompt — codex's separator above the
        // footer.
        for line in ((prompt_line + 1)..end).rev() {
            if !nonblank[line] {
                end = line;
                break;
            }
        }
    }
    end
}

/// Read `region` as `tool`'s input box — `_input_region_occupied`.
#[must_use]
pub fn occupancy(region: &str, model: InputModel) -> Occupancy {
    if region.is_empty() {
        return Occupancy::Unreadable;
    }
    let segments = parse(region);
    match model {
        InputModel::StyleDelimited => {
            // The live prompt is the bottom-most row whose first non-blank cell
            // is `›` in BOLD-and-NOT-DIM state; a submitted transcript echo is
            // the same ornament bold AND dim.
            let Some(found) = prompt(&segments, true, &["›"]) else {
                return Occupancy::Unreadable;
            };
            let end = content_end(&segments, segments[found.index].line, StopAt::Blank);
            // Content is everything after the ANCHOR ornament only.
            let mut text = after_first(&found.tail, '›');
            for seg in &segments[found.index + 1..] {
                if seg.line >= end {
                    break;
                }
                if seg.dim {
                    continue; // the placeholder suggestion, not user content
                }
                text.push_str(&seg.text);
            }
            // ANY unstyled printable remainder is OCCUPIED — including the
            // unstyled `[Pasted Content N chars]` staging token.
            verdict(&text)
        }
        InputModel::BorderDelimited => {
            // STRUCTURE, because styling cannot identify claude's live prompt:
            // the submitted echo, the idle prompt and the mid-generation prompt
            // differ in colour and NONE of them is SGR-dim.
            let Some(found) = prompt(&segments, false, &["❯", ">", "▌"]) else {
                return Occupancy::Unreadable;
            };
            let end = content_end(&segments, segments[found.index].line, StopAt::Border);
            let mut text = found
                .tail
                .strip_prefix(found.ornament)
                .unwrap_or(&found.tail)
                .to_owned();
            // Continuation rows of a multiline draft sit BELOW the prompt row,
            // including below the edit cursor, and are real unsent input.
            for seg in &segments[found.index + 1..] {
                if seg.line >= end {
                    break;
                }
                text.push_str(&seg.text);
            }
            verdict(&text)
        }
        // No grammar for this box: ae read NOTHING, so it claims nothing. Idle
        // would be a claim it cannot honour. The callers that decide whether a
        // paste may proceed short-circuit unmodelled BEFORE this
        // (`input_busy`, `still_staged`), and the one caller left
        // (`clear_is_measurable`, notice proof only) fails closed on the
        // unreadable verdict.
        InputModel::Unmodelled => Occupancy::Unreadable,
    }
}

/// Does `capture` show a COMPOSED input box carrying one of `markers`?
///
/// The BOTTOM-MOST box owns the answer — a transcript echo or a modal higher
/// on the screen is not the input, the same bottom-most-owner rule
/// [`composer_row`] enforces for the modelled tools. The box is recognised by
/// the measured `opencode` 1.18.31 geometry: rows whose first non-blank cell
/// is the `┃` rail, closed by an edge row starting `╹▀`. A marker counts only
/// INSIDE those rows, so a marker in a transcript echo, in scrollback or in a
/// modal without a drawn box is never readiness. An empty marker list answers
/// false: a tool with no usable composed signal is refused, not guessed.
///
/// The geometry is one tool's measured shape; a future unmodelled tool whose
/// box is drawn differently needs its own detector, not this one's markers.
#[must_use]
pub fn composed_ui(capture: &str, markers: &[&str]) -> bool {
    if markers.is_empty() {
        return false;
    }
    // A capture may be plain or SGR-styled: join each row's segments first, so
    // the box geometry and the markers are read from TEXT, never from the
    // escape bytes a styled capture interleaves.
    let segments = parse(capture);
    let mut rows: Vec<String> = Vec::new();
    for seg in &segments {
        if rows.len() <= seg.line {
            rows.resize(seg.line + 1, String::new());
        }
        if let Some(row) = rows.get_mut(seg.line) {
            row.push_str(&seg.text);
        }
    }
    let Some(edge) = rows.iter().rposition(|row| is_composer_edge(row)) else {
        return false;
    };
    // The box: its edge plus the contiguous rail rows directly above it.
    let top = (0..edge)
        .rev()
        .find(|&at| !is_composer_rail(&rows[at]))
        .map_or(0, |at| at + 1);
    rows[top..=edge]
        .iter()
        .any(|row| markers.iter().any(|marker| row.contains(marker)))
}

/// A composer box's left rail: the row's first non-blank cell is `┃`.
fn is_composer_rail(row: &str) -> bool {
    row.trim_start_matches(is_space).starts_with('┃')
}

/// A composer box's bottom edge: the row's first non-blank cell is the `╹`
/// corner, directly followed by the `▀` underline run.
fn is_composer_edge(row: &str) -> bool {
    let mut cells = row.trim_start_matches(is_space).chars();
    cells.next() == Some('╹') && cells.next() == Some('▀')
}

/// Does Claude's live prompt say it accepted a message into its turn queue?
///
/// The phrase is an affordance the TUI draws after a mid-turn submit, not
/// draft text. It must therefore never make submit verification retry Enter.
#[must_use]
pub fn queued_submission(region: &str, model: InputModel) -> bool {
    if model != InputModel::BorderDelimited {
        return false;
    }
    let segments = parse(region);
    let Some(found) = prompt(&segments, false, &["❯", ">", "▌"]) else {
        return false;
    };
    found
        .tail
        .strip_prefix(found.ornament)
        .is_some_and(|text| text.trim_start_matches(is_space) == "Press up to edit queued messages")
}

/// Everything after the first `needle` in `text`, or all of it when there is
/// none.
fn after_first(text: &str, needle: char) -> String {
    match text.find(needle) {
        Some(at) => text[at + needle.len_utf8()..].to_owned(),
        None => text.to_owned(),
    }
}

/// Whether what was gathered from the box counts as content.
fn verdict(text: &str) -> Occupancy {
    let stripped: String = text
        .replace('\u{a0}', " ")
        .chars()
        .filter(|ch| !matches!(ch, '\n' | '\t' | ' '))
        .collect();
    if stripped.is_empty() {
        Occupancy::Idle
    } else {
        Occupancy::Occupied
    }
}

/// Is `capture` a codex that is PROVABLY still starting up —
/// `_tool_initializing`?
#[must_use]
pub fn initializing(capture: &str, model: InputModel) -> bool {
    if model != InputModel::StyleDelimited || capture.is_empty() {
        return false;
    }
    capture
        .lines()
        .any(|line| mcp_progress(line) || model_loading(line))
}

/// `^•[[:space:]]+Starting MCP servers \([0-9]+/[0-9]+\)` — the bullet the TUI
/// draws starts the line, and the counter is what makes it a class.
fn mcp_progress(line: &str) -> bool {
    let Some(after) = line.strip_prefix('•') else {
        return false;
    };
    let rest = after.trim_start_matches(is_space);
    if rest.len() == after.len() {
        return false; // at least one space is required
    }
    let Some(rest) = rest.strip_prefix("Starting MCP servers (") else {
        return false;
    };
    let (count, rest) = digits(rest);
    if count == 0 {
        return false;
    }
    let Some(rest) = rest.strip_prefix('/') else {
        return false;
    };
    let (count, rest) = digits(rest);
    count > 0 && rest.starts_with(')')
}

/// `^│[[:space:]]*model:[[:space:]]+loading([[:space:]].*)?/model to change`
/// — inside the box, carrying the live affordance beside the value.
fn model_loading(line: &str) -> bool {
    /// The live affordance the header row carries beside its value.
    const AFFORDANCE: &str = "/model to change";
    let Some(rest) = line.strip_prefix('│') else {
        return false;
    };
    let Some(rest) = rest.trim_start_matches(is_space).strip_prefix("model:") else {
        return false;
    };
    let after = rest.trim_start_matches(is_space);
    if after.len() == rest.len() {
        return false; // at least one space is required
    }
    let Some(rest) = after.strip_prefix("loading") else {
        return false;
    };
    if rest.starts_with(AFFORDANCE) {
        return true;
    }
    rest.starts_with(is_space) && rest.contains(AFFORDANCE)
}

/// A POSIX `[[:space:]]` character.
fn is_space(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r')
}

/// Whether every character is a POSIX space.
pub(super) fn is_blank(text: &str) -> bool {
    text.chars().all(is_space)
}

/// `text` with POSIX spaces trimmed from both ends.
pub(super) fn trim_posix(text: &str) -> &str {
    text.trim_matches(is_space)
}

/// How many ASCII digits lead `text`, and what follows them.
fn digits(text: &str) -> (usize, &str) {
    let at = text
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(text.len());
    (at, &text[at..])
}

#[cfg(test)]
mod tests {
    use super::{
        Occupancy, Segment, composed_ui, initializing, occupancy, parse, prompt, queued_submission,
    };
    use crate::tool::InputModel;

    /// The REAL stuck composer — muse-spark-1.3 via the muse CLI, captured
    /// 2026-09-14. The harness REFUSED the pasted turn and its composer still
    /// holds the staged token; provenance is beside the file.
    const MUSE_STUCK: &str =
        include_str!("../../tests/fixtures/muse-composer/muse-stuck-composer.esc");
    /// Staged, before Enter, from a live ae-dev seat — the same full-composer
    /// state as the stuck frame, with transcript echoes of earlier turns.
    const MUSE_OCCUPIED: &str =
        include_str!("../../tests/fixtures/muse-composer/muse-occupied-composer.esc");
    /// ACCEPTED: the composer cleared and the staged token moved into the
    /// TRANSCRIPT. Three `❯` rows on screen; only the bottom one is the box.
    const MUSE_ACCEPTED: &str =
        include_str!("../../tests/fixtures/muse-composer/muse-accepted-composer.esc");
    /// Nothing staged; a backend 429 sits in the transcript, composer empty.
    const MUSE_IDLE: &str =
        include_str!("../../tests/fixtures/muse-composer/muse-idle-composer.esc");

    /// The REAL opencode composed frame and its blank boot frame (provenance
    /// beside them). The marker is the one the adapter pins for 1.18.31.
    const OPENCODE_COMPOSED: &str =
        include_str!("../../tests/fixtures/opencode-composer/opencode-composed-frame.esc");
    const OPENCODE_BOOT: &str =
        include_str!("../../tests/fixtures/opencode-composer/opencode-boot-frame.esc");
    const OPENCODE_MARKERS: &[&str] = &["Ask anything…"];

    /// A row as `capture-pane -e` renders it: the styling matters, and it is
    /// spelled the way tmux legally spells it rather than one canonical way.
    fn seg(bold: bool, dim: bool, text: &str, line: usize) -> Segment {
        Segment {
            bold,
            dim,
            text: text.to_owned(),
            line,
        }
    }

    #[test]
    fn sgr_state_is_parsed_not_matched_and_a_segment_never_spans_a_row() {
        // The SAME visual state, three legal serialisations.
        for spelling in ["\u{1b}[1m›", "\u{1b}[0;1m›", "\u{1b}[2m…\u{1b}[0;1m›"] {
            let parsed = parse(spelling);
            let last = parsed.last().expect("a segment");
            assert_eq!(
                (last.bold, last.dim, last.text.as_str()),
                (true, false, "›"),
                "{spelling:?}"
            );
        }
        // …and one that leaves DIM active, which is the opposite meaning.
        let echo = parse("\u{1b}[2m…\u{1b}[1m›");
        let last = echo.last().expect("a segment");
        assert_eq!(
            (last.bold, last.dim),
            (true, true),
            "bold AND dim is an echo"
        );
        assert_eq!(
            parse("a\nb\nc"),
            vec![
                seg(false, false, "a", 0),
                seg(false, false, "b", 1),
                seg(false, false, "c", 2)
            ],
            "a run split at every newline, one row each"
        );
    }

    #[test]
    fn an_extended_colour_introducer_does_not_leak_a_bold_or_dim_parameter() {
        let parsed = parse("\u{1b}[38;5;1mx");
        assert_eq!(
            (parsed[0].bold, parsed[0].dim),
            (false, false),
            "38;5;N — the 1 is a colour index, not bold"
        );
        let truecolour = parse("\u{1b}[38;2;1;2;3mx");
        assert_eq!((truecolour[0].bold, truecolour[0].dim), (false, false));
        let after = parse("\u{1b}[38;5;1;1mx");
        assert!(
            after[0].bold,
            "a real 1 AFTER the skipped arguments still counts"
        );
    }

    #[test]
    fn terminal_control_that_is_not_sgr_is_consumed_rather_than_read_as_text() {
        // An OSC hyperlink is zero-width control data; leaving it in would
        // inflate a row until the border looked like occupied content.
        assert_eq!(
            parse("\u{1b}]8;;http://x\u{7}link"),
            vec![seg(false, false, "link", 0)]
        );
        assert_eq!(
            parse("\u{1b}]8;;http://x\u{1b}\\link"),
            vec![seg(false, false, "link", 0)]
        );
        // An UNTERMINATED OSC must not eat the next row.
        assert_eq!(
            parse("\u{1b}]8;;open\nnext"),
            vec![seg(false, false, "next", 1)],
            "the row boundary ends it, and the counter advances"
        );
        // A charset designator's FINAL byte must not leak into a text run: a
        // stray `B` reads as content, and a false OCCUPIED is the duplication
        // class.
        assert_eq!(parse("\u{1b}(Bx"), vec![seg(false, false, "x", 0)]);
        // A non-SGR CSI is consumed with no state change.
        assert_eq!(parse("\u{1b}[2Kx"), vec![seg(false, false, "x", 0)]);
    }

    /// A claude frame: transcript, the prompt row, the box's bottom border,
    /// then the status rows below it.
    fn claude_frame(input: &str) -> String {
        let border = "─".repeat(60);
        format!("transcript row\n\u{1b}[1m❯\u{1b}[0m\u{a0}{input}\n{border}\n  model  ~/x\n")
    }

    #[test]
    fn claude_is_idle_on_a_bare_ornament_and_occupied_on_anything_else() {
        assert_eq!(
            occupancy(&claude_frame(""), InputModel::BorderDelimited),
            Occupancy::Idle
        );
        assert_eq!(
            occupancy(
                &claude_frame("[Pasted text #1 +40 lines]"),
                InputModel::BorderDelimited
            ),
            Occupancy::Occupied,
            "the staging token is just content — the CLASS is the rule, not the text"
        );
        assert_eq!(
            occupancy(&claude_frame("half a que"), InputModel::BorderDelimited),
            Occupancy::Occupied
        );
        assert_eq!(
            occupancy("nothing but transcript\n", InputModel::BorderDelimited),
            Occupancy::Unreadable,
            "no live prompt in view is INDETERMINATE, never idle"
        );
        assert_eq!(
            occupancy("", InputModel::BorderDelimited),
            Occupancy::Unreadable
        );
    }

    #[test]
    fn a_multiline_draft_below_the_prompt_row_is_still_input() {
        let border = "─".repeat(60);
        let region =
            format!("transcript\n\u{1b}[1m❯\u{1b}[0m\u{a0}\nstill unsent\n{border}\n  model\n");
        assert_eq!(
            occupancy(&region, InputModel::BorderDelimited),
            Occupancy::Occupied,
            "a draft whose first row is blank keeps its real rows BELOW the prompt"
        );
    }

    #[test]
    fn a_draft_row_that_merely_starts_with_the_border_glyph_does_not_truncate_the_box() {
        // The shipped clobber: a user pastes `─ heading` into a draft, a
        // first-cell test truncates the region there, and the unsent body
        // below it reads IDLE.
        let border = "─".repeat(60);
        let region = format!(
            "transcript\n\u{1b}[1m❯\u{1b}[0m\u{a0}\n─ user-pasted heading\nreal unsent text\n{border}\n  model\n"
        );
        assert_eq!(
            occupancy(&region, InputModel::BorderDelimited),
            Occupancy::Occupied,
            "morphology AND width, together: that row is neither"
        );
    }

    /// A codex frame: the live prompt is BOLD-and-not-dim, the box ends at a
    /// BLANK row, and the footer sits under it.
    fn codex_frame(prompt: &str, input: &str) -> String {
        format!("\u{1b}[1;2m› \u{1b}[0man earlier, submitted line\n{prompt}{input}\n\n  gpt  ~/x\n")
    }

    #[test]
    fn codex_reads_its_live_prompt_by_style_and_ignores_its_submitted_echo() {
        assert_eq!(
            occupancy(
                &codex_frame("\u{1b}[1m›\u{1b}[0m ", ""),
                InputModel::StyleDelimited
            ),
            Occupancy::Idle
        );
        assert_eq!(
            occupancy(
                &codex_frame("\u{1b}[1m›\u{1b}[0m ", "[Pasted Content 1469 chars]"),
                InputModel::StyleDelimited
            ),
            Occupancy::Occupied,
            "the unstyled staging token is content — the read that once said clear"
        );
        assert_eq!(
            occupancy(
                &codex_frame(
                    "\u{1b}[1m›\u{1b}[0m ",
                    "\u{1b}[2mtry \"explain this\"\u{1b}[0m"
                ),
                InputModel::StyleDelimited
            ),
            Occupancy::Idle,
            "a DIM placeholder suggestion is not user content"
        );
        assert_eq!(
            occupancy(
                "\u{1b}[1;2m› \u{1b}[0mjust a submitted echo\n\n  gpt\n",
                InputModel::StyleDelimited
            ),
            Occupancy::Unreadable,
            "bold AND dim is an echo, not a live prompt"
        );
        assert_eq!(
            occupancy(
                &codex_frame("\u{1b}[1m›\u{1b}[0m ", "a › typed by the user"),
                InputModel::StyleDelimited
            ),
            Occupancy::Occupied,
            "only the ANCHOR ornament is stripped; a typed one is content"
        );
    }

    #[test]
    fn an_unmodelled_tool_reads_unreadable_and_is_kept_unblocked_by_its_callers() {
        assert_eq!(
            occupancy(&claude_frame("busy"), InputModel::Unmodelled),
            Occupancy::Unreadable,
            "a box ae cannot read is never a claim of Idle"
        );
        assert!(!InputModel::Unmodelled.is_modelled() && InputModel::BorderDelimited.is_modelled());
    }

    #[test]
    fn the_start_up_markers_are_rows_the_tui_draws_not_text_on_the_screen() {
        let progress = "• Starting MCP servers (0/7): assistant-all-tools\n";
        let header = "│ model:       loading   /model to change │\n";
        assert!(initializing(progress, InputModel::StyleDelimited));
        assert!(initializing(header, InputModel::StyleDelimited));
        assert!(initializing(
            &format!("chrome\n{progress}box\n"),
            InputModel::StyleDelimited
        ));
        // A QUOTED frame is indented, and that is the whole difference: these
        // strings are in this project's own docs, so a substring scan reads
        // "initializing" forever in the pane of an agent reading them.
        assert!(!initializing(
            &format!("  {progress}"),
            InputModel::StyleDelimited
        ));
        assert!(!initializing(
            &format!("  {header}"),
            InputModel::StyleDelimited
        ));
        // An ASCII pipe is a markdown table row, not the TUI's box.
        assert!(!initializing(
            "| model:       loading   /model to change |\n",
            InputModel::StyleDelimited
        ));
        // The counter is what makes the progress row a CLASS.
        assert!(!initializing(
            "• Starting MCP servers (): x\n",
            InputModel::StyleDelimited
        ));
        assert!(!initializing(
            "•Starting MCP servers (0/7): x\n",
            InputModel::StyleDelimited
        ));
        // Settled: the value is a model, not `loading`.
        assert!(!initializing(
            "│ model:       gpt-5.6 xhigh   /model to change │\n",
            InputModel::StyleDelimited
        ));
        // And the markers are CODEX's.
        assert!(!initializing(progress, InputModel::BorderDelimited));
        assert!(!initializing(progress, InputModel::Unmodelled));
        assert!(!initializing("", InputModel::StyleDelimited));
    }

    #[test]
    fn a_transcript_echo_of_a_submitted_turn_does_not_read_as_still_staged() {
        // `❯` is NOT unique on screen: the accepted and occupied frames each
        // carry three ornament rows, and one transcript echo still carries the
        // staged token. Only the composer — the bottom-most ornament row,
        // bounded downward by the box's rule — is the input box. A containment
        // test would read the ACCEPTED turn as staged and duplicate it.
        assert!(MUSE_ACCEPTED.matches('❯').count() >= 3);
        assert!(MUSE_ACCEPTED.contains("[Pasted Content 1766 chars]"));
        assert_eq!(
            occupancy(MUSE_ACCEPTED, InputModel::BorderDelimited),
            Occupancy::Idle,
            "the token in the TRANSCRIPT is not staged content"
        );
        assert_eq!(
            occupancy(MUSE_OCCUPIED, InputModel::BorderDelimited),
            Occupancy::Occupied
        );
        assert_eq!(
            occupancy(MUSE_IDLE, InputModel::BorderDelimited),
            Occupancy::Idle,
            "a backend error in the transcript does not keep the box occupied"
        );
        assert!(!queued_submission(
            MUSE_ACCEPTED,
            InputModel::BorderDelimited
        ));
        assert!(!queued_submission(
            MUSE_OCCUPIED,
            InputModel::BorderDelimited
        ));
        assert!(!queued_submission(MUSE_IDLE, InputModel::BorderDelimited));
    }

    #[test]
    fn a_muse_composer_that_refused_the_turn_still_reads_occupied() {
        // The real refusal: the box did NOT clear, so the EXISTING retry loop
        // (still_staged -> StillStaged -> another Enter) is what answers the
        // harness's own "try again" — no vendor string is matched.
        assert_eq!(
            occupancy(MUSE_STUCK, InputModel::BorderDelimited),
            Occupancy::Occupied
        );
        assert!(
            !queued_submission(MUSE_STUCK, InputModel::BorderDelimited),
            "claude's queue affordance is inert on muse"
        );
    }

    #[test]
    fn the_muse_prompt_is_the_composer_row_and_its_bottom_rule_bounds_the_box() {
        let segments = parse(MUSE_STUCK);
        let found = prompt(&segments, false, &["❯", ">", "▌"]).expect("the live composer");
        let composer_line = segments[found.index].line;
        assert!(found.tail.starts_with('❯'));
        assert!(
            segments
                .iter()
                .any(|seg| seg.line == composer_line
                    && seg.text.contains("[Pasted Content 1494 chars]")),
            "the found prompt row is the COMPOSER row: line {composer_line}"
        );

        // A transcript echo carrying the SAME ornament must not capture the
        // read: prompt() scans bottom-up, and the composer sits below every
        // transcript row. (The captured frame itself carries one `❯`.)
        let with_echo = format!("❯ a transcript echo\n{MUSE_STUCK}");
        let echoed = parse(&with_echo);
        let picked = prompt(&echoed, false, &["❯", ">", "▌"]).expect("still the live composer");
        let picked_line = echoed[picked.index].line;
        assert!(
            echoed
                .iter()
                .any(|seg| seg.line == picked_line
                    && seg.text.contains("[Pasted Content 1494 chars]")),
            "the echo above does not capture the read; picked line {picked_line}"
        );
        assert!(picked_line > 0, "the echo really is above the composer");

        // The same captured frame with the staged token removed. It reads Idle
        // only if content_end stops at muse's bottom rule — otherwise the
        // footer BELOW the box would read as content. The real ACCEPTED frame
        // proves the cleared leg; this isolates the geometry.
        let cleared = MUSE_STUCK.replace("[Pasted Content 1494 chars]", "");
        assert_eq!(
            occupancy(&cleared, InputModel::BorderDelimited),
            Occupancy::Idle
        );
    }

    #[test]
    fn a_composed_marker_counts_only_inside_the_bottom_composer_box() {
        // The real frames: the marker lives inside the drawn box, and the
        // blank boot frame has no box at all.
        assert!(composed_ui(OPENCODE_COMPOSED, OPENCODE_MARKERS));
        assert!(
            composed_ui(OPENCODE_COMPOSED, &["Build", "ctrl+p commands"]),
            "any marker inside the box answers; the box is the anchor"
        );
        assert!(!composed_ui(OPENCODE_BOOT, OPENCODE_MARKERS));

        // A transcript echo carrying the marker with NO composer drawn is not
        // readiness — the whole-capture `contains` this replaces said it was.
        let echoed = "❯ Ask anything… quoted from an earlier turn\nplain transcript\n";
        assert!(!composed_ui(echoed, OPENCODE_MARKERS));

        // A marker visible ABOVE a drawn box that does not carry it: the box
        // owns the answer, so the echo above it must not grant readiness. The
        // same box does compose for a marker inside it.
        let boxed = "❯ Ask anything… quoted from an earlier turn\n\n   ┃\n   ┃  write here\n   ╹▀▀▀▀▀\n   tab agents\n";
        assert!(!composed_ui(boxed, OPENCODE_MARKERS));
        assert!(composed_ui(boxed, &["write here"]));

        // A tool with no usable composed signal never composes anything.
        assert!(!composed_ui(OPENCODE_COMPOSED, &[]));
    }
}
