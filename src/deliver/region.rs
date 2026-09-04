//! The input-region sensor: what is in a TUI's input box right now.
//!
//! Ported from `ae`'s `_sgr_parse`, `_input_region_prompt_seg`,
//! `_input_region_content_end`, `_input_region_is_border` and
//! `_input_region_occupied` — the pane-reading half of the delivery path, and
//! the half where every shipped delivery bug lived. Nothing here runs tmux:
//! it takes a capture and answers a question about it, so the whole model is
//! unit-testable against recorded frames.

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

/// The tools whose input box ae models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// Claude Code.
    Claude,
    /// Codex.
    Codex,
    /// Anything else — unmodelled.
    Other,
}

impl Tool {
    /// The tool a `pane_current_command` or a recorded `agent_bin` names.
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name {
            "claude" => Self::Claude,
            "codex" => Self::Codex,
            _ => Self::Other,
        }
    }

    /// The name the frozen `ae_target_tool` printed, which is what every
    /// diagnostic line in this path interpolates.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Other => "other",
        }
    }

    /// Whether ae models this tool's input box at all.
    #[must_use]
    pub fn is_modelled(self) -> bool {
        !matches!(self, Self::Other)
    }
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
                // No final byte at all: the frozen parser consumes the whole
                // remainder rather than spinning on it.
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

/// Find the live input prompt — `_input_region_prompt_seg`.
fn prompt(segments: &[Segment], live_only: bool, ornaments: &[&'static str]) -> Option<Prompt> {
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
            return Some(Prompt {
                index,
                tail: tail.to_owned(),
                ornament,
            });
        }
    }
    None
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
pub fn occupancy(region: &str, tool: Tool) -> Occupancy {
    if region.is_empty() {
        return Occupancy::Unreadable;
    }
    let segments = parse(region);
    match tool {
        Tool::Codex => {
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
        Tool::Claude => {
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
        Tool::Other => Occupancy::Idle,
    }
}

/// Everything after the first `needle` in `text`, or all of it when there is
/// none — the frozen `${tail#*›}`.
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
pub fn initializing(capture: &str, tool: Tool) -> bool {
    if tool != Tool::Codex || capture.is_empty() {
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

/// Does this capture show a composed TUI — the frozen spawn readiness grep
/// for a tool ae does not model, `'❯|bypass permissions|for shortcuts'`.
#[must_use]
pub fn composed_ui(capture: &str) -> bool {
    capture.contains('❯')
        || capture.contains("bypass permissions")
        || capture.contains("for shortcuts")
}

/// A POSIX `[[:space:]]` character.
fn is_space(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r')
}

/// Whether every character is a POSIX space — the frozen
/// `[[ -z "${text//[[:space:]]/}" ]]`.
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
    use super::{Occupancy, Segment, Tool, composed_ui, initializing, occupancy, parse};

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
        assert_eq!(occupancy(&claude_frame(""), Tool::Claude), Occupancy::Idle);
        assert_eq!(
            occupancy(&claude_frame("[Pasted text #1 +40 lines]"), Tool::Claude),
            Occupancy::Occupied,
            "the staging token is just content — the CLASS is the rule, not the text"
        );
        assert_eq!(
            occupancy(&claude_frame("half a que"), Tool::Claude),
            Occupancy::Occupied
        );
        assert_eq!(
            occupancy("nothing but transcript\n", Tool::Claude),
            Occupancy::Unreadable,
            "no live prompt in view is INDETERMINATE, never idle"
        );
        assert_eq!(occupancy("", Tool::Claude), Occupancy::Unreadable);
    }

    #[test]
    fn a_multiline_draft_below_the_prompt_row_is_still_input() {
        let border = "─".repeat(60);
        let region =
            format!("transcript\n\u{1b}[1m❯\u{1b}[0m\u{a0}\nstill unsent\n{border}\n  model\n");
        assert_eq!(
            occupancy(&region, Tool::Claude),
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
            occupancy(&region, Tool::Claude),
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
            occupancy(&codex_frame("\u{1b}[1m›\u{1b}[0m ", ""), Tool::Codex),
            Occupancy::Idle
        );
        assert_eq!(
            occupancy(
                &codex_frame("\u{1b}[1m›\u{1b}[0m ", "[Pasted Content 1469 chars]"),
                Tool::Codex
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
                Tool::Codex
            ),
            Occupancy::Idle,
            "a DIM placeholder suggestion is not user content"
        );
        assert_eq!(
            occupancy(
                "\u{1b}[1;2m› \u{1b}[0mjust a submitted echo\n\n  gpt\n",
                Tool::Codex
            ),
            Occupancy::Unreadable,
            "bold AND dim is an echo, not a live prompt"
        );
        assert_eq!(
            occupancy(
                &codex_frame("\u{1b}[1m›\u{1b}[0m ", "a › typed by the user"),
                Tool::Codex
            ),
            Occupancy::Occupied,
            "only the ANCHOR ornament is stripped; a typed one is content"
        );
    }

    #[test]
    fn an_unmodelled_tool_is_idle_so_delivery_to_it_is_never_blocked() {
        assert_eq!(
            occupancy(&claude_frame("busy"), Tool::Other),
            Occupancy::Idle
        );
        assert_eq!(Tool::from_name("opencode.exe"), Tool::Other);
        assert_eq!(Tool::from_name("claude"), Tool::Claude);
        assert_eq!(Tool::from_name("codex"), Tool::Codex);
        assert_eq!(Tool::Other.as_str(), "other");
        assert!(!Tool::Other.is_modelled() && Tool::Claude.is_modelled());
    }

    #[test]
    fn an_unmodelled_tool_keeps_the_marker_grep_it_always_had() {
        assert!(composed_ui("some box\n❯ \n"));
        assert!(composed_ui("? for shortcuts"));
        assert!(composed_ui("bypass permissions on"));
        assert!(!composed_ui(""));
        assert!(!composed_ui("still booting\n"));
    }

    #[test]
    fn the_start_up_markers_are_rows_the_tui_draws_not_text_on_the_screen() {
        let progress = "• Starting MCP servers (0/7): assistant-all-tools\n";
        let header = "│ model:       loading   /model to change │\n";
        assert!(initializing(progress, Tool::Codex));
        assert!(initializing(header, Tool::Codex));
        assert!(initializing(
            &format!("chrome\n{progress}box\n"),
            Tool::Codex
        ));
        // A QUOTED frame is indented, and that is the whole difference: these
        // strings are in this project's own docs, so a substring scan reads
        // "initializing" forever in the pane of an agent reading them.
        assert!(!initializing(&format!("  {progress}"), Tool::Codex));
        assert!(!initializing(&format!("  {header}"), Tool::Codex));
        // An ASCII pipe is a markdown table row, not the TUI's box.
        assert!(!initializing(
            "| model:       loading   /model to change |\n",
            Tool::Codex
        ));
        // The counter is what makes the progress row a CLASS.
        assert!(!initializing("• Starting MCP servers (): x\n", Tool::Codex));
        assert!(!initializing(
            "•Starting MCP servers (0/7): x\n",
            Tool::Codex
        ));
        // Settled: the value is a model, not `loading`.
        assert!(!initializing(
            "│ model:       gpt-5.6 xhigh   /model to change │\n",
            Tool::Codex
        ));
        // And the markers are CODEX's.
        assert!(!initializing(progress, Tool::Claude));
        assert!(!initializing(progress, Tool::Other));
        assert!(!initializing("", Tool::Codex));
    }
}
