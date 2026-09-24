//! Pure classification of the current harness frame already captured by the watchdog.
//!
//! Shared residual, accepted for all four bottom-anchored identity grammars
//! together: a QUOTED frame that ends exactly at the tool's live geometry —
//! the same bottom rows in the same order — while the tool's own composer is
//! absent reads as observed, because a plain capture carries no provenance.
//! The obligation is on the wiring slice: an identity read from a capture
//! whose tool-composed signal is false must not be trusted.

use std::borrow::Cow;

use crate::tool::{IdentitySpec, InputModel, ToolKind};

/// What the current, positively recognized harness frame says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HarnessState {
    /// The harness is executing a turn.
    Busy,
    /// The harness is waiting at its empty input box.
    Idle,
    /// The frame is unsupported, incomplete, transitional, or otherwise ambiguous.
    #[default]
    Unknown,
}

/// Independently observed identity fields from the current harness frame.
///
/// A missing field is deliberately different from a guessed value: the frame
/// may prove effort while its model label is unfamiliar (or vice versa).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HarnessIdentity {
    pub model: Option<String>,
    pub effort: Option<String>,
}

impl HarnessState {
    /// The stable low-cardinality spelling published to tmux and `ae list`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::Idle => "idle",
            Self::Unknown => "unknown",
        }
    }
}

/// The idle episode carried in the watchdog-owned observed pane option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleCarry {
    /// First positive idle observation, as epoch seconds.
    pub since_epoch: i64,
    /// Successful reminder deliveries in this episode.
    pub nudges: u32,
    /// Consecutive reminder attempts that could not be delivered.
    pub undelivered: u32,
    /// Stable slot+agent identity hash guarding pane-id reuse.
    pub identity: u64,
    /// Stable fingerprint of the newest declaration already applied.
    pub declaration: Option<u64>,
}

/// Read the public state from a watchdog-owned pane option.
///
/// Malformed carry is `Unknown`, never an assertion that the harness is idle.
#[must_use]
pub fn observed_from_option(value: &str) -> HarnessState {
    match value {
        "busy" => HarnessState::Busy,
        "unknown" => HarnessState::Unknown,
        "idle" => HarnessState::Idle,
        value => decode_carry(value).map_or(HarnessState::Unknown, |(frame, _)| frame),
    }
}

/// Encode one idle episode without adding another tmux observation door.
#[must_use]
pub fn encode_idle(carry: IdleCarry) -> String {
    encode_carry(HarnessState::Idle, carry)
}

/// Encode an idle episode while retaining the current public frame.
#[must_use]
pub(crate) fn encode_carry(frame: HarnessState, carry: IdleCarry) -> String {
    let frame = match frame {
        HarnessState::Idle => "idle",
        HarnessState::Unknown => "unknown",
        HarnessState::Busy => return HarnessState::Busy.as_str().to_owned(),
    };
    format!(
        "{frame}:{}:{}:{}:{}:{}",
        carry.since_epoch,
        carry.nudges,
        carry.undelivered,
        carry.identity,
        carry
            .declaration
            .map_or_else(|| "-".to_owned(), |value| value.to_string()),
    )
}

/// Recover a well-formed idle episode after a watchdog restart.
#[must_use]
pub fn decode_idle(value: &str) -> Option<IdleCarry> {
    decode_carry(value).map(|(_, carry)| carry)
}

fn decode_carry(value: &str) -> Option<(HarnessState, IdleCarry)> {
    let mut fields = value.split(':');
    let frame = match fields.next()? {
        "idle" => HarnessState::Idle,
        "unknown" => HarnessState::Unknown,
        _ => return None,
    };
    let carry = IdleCarry {
        since_epoch: fields.next()?.parse().ok()?,
        nudges: fields.next()?.parse().ok()?,
        undelivered: fields.next()?.parse().ok()?,
        identity: fields.next()?.parse().ok()?,
        declaration: match fields.next() {
            None | Some("-") => None,
            Some(value) => Some(value.parse().ok()?),
        },
    };
    (fields.next().is_none() && carry.since_epoch >= 0).then_some((frame, carry))
}

/// Classify only a positively recognized current harness frame.
#[must_use]
pub fn classify(capture: &str, tool: ToolKind) -> HarnessState {
    classify_model(capture, tool.input_model())
}

fn classify_model(capture: &str, model: InputModel) -> HarnessState {
    match model {
        InputModel::BorderDelimited => classify_claude(capture),
        InputModel::StyleDelimited => classify_codex(capture),
        InputModel::Unmodelled => HarnessState::Unknown,
    }
}

/// Whether the current modeled input box contains a human draft.
#[must_use]
pub fn has_human_draft(capture: &str, tool: ToolKind) -> bool {
    let model = tool.input_model();
    match model {
        InputModel::StyleDelimited => {
            let Some(frame) = codex_frame(capture) else {
                return false;
            };
            codex_footer(&frame.footer)
                && !frame.above.as_deref().is_some_and(codex_modal)
                && !codex_placeholder(&frame.prompt)
                && frame
                    .prompt
                    .strip_prefix('›')
                    .is_some_and(|input| !crate::deliver::region::is_furniture(input))
        }
        InputModel::BorderDelimited => {
            let lines = clean_lines(capture);
            lines
                .iter()
                .rposition(|line| line.starts_with('❯'))
                .is_some_and(|index| claude_input_frame(&lines, index) && lines[index] != "❯")
        }
        InputModel::Unmodelled => false,
    }
}

/// Extract model and effort independently from the positively bounded current
/// frame. This never searches transcript text or historical rows.
#[must_use]
pub fn current_identity(capture: &str, tool: ToolKind) -> HarnessIdentity {
    match tool.adapter().identity {
        IdentitySpec::BorderComposer => current_claude_identity(capture),
        IdentitySpec::StyleFooter => current_codex_identity(capture),
        IdentitySpec::RuleFooter => current_muse_identity(capture),
        IdentitySpec::RailStatus => current_opencode_identity(capture),
        IdentitySpec::BorderText => current_grok_identity(capture),
        IdentitySpec::TrailingLabel => current_agy_identity(capture),
        // Gemini CLI has no measured frame, and no other tool's frame
        // grammar is modelled: unknown, never a guess.
        IdentitySpec::Unmodelled => HarnessIdentity::default(),
    }
}

/// The identity this capture proves, gated on the tool's own live-composer
/// signal in the SAME capture — the module doc's WIRING OBLIGATION.
///
/// A caller that will SHOW the answer takes this; the drift observer keeps
/// the ungated [`current_identity`], because those are different questions:
/// what the frame says, versus what this frame currently is.
#[must_use]
pub fn observed_identity(capture: &str, tool: ToolKind) -> HarnessIdentity {
    if composer_present(capture, tool) {
        current_identity(capture, tool)
    } else {
        HarnessIdentity::default()
    }
}

/// Whether `capture` proves this tool's own composer is drawn on it.
///
/// Per identity class, because the classes differ in what they already prove:
///
/// - claude and codex need NOTHING added: their grammars already require the
///   live input box in the same capture. Asking [`crate::deliver::region`]
///   again would also be WRONG for codex, whose `StyleDelimited` composer is
///   found by SGR weight — the watchdog captures PLAIN, so that read answers
///   `Unreadable` on every live frame.
/// - muse draws a modelled box but carries no composed markers
///   ([`crate::tool::Composed::NONE`]), so `composed_ui` can never pass for
///   it. Its own input box is the signal instead, read STRUCTURALLY, which is
///   what survives a plain capture.
/// - opencode, grok and agy are unmodelled composers with measured markers,
///   which is what [`crate::deliver::region::composed_ui`] answers for, from
///   the adapter row as DATA rather than a per-tool branch here.
///
/// Named residual: for muse the ornament row this finds may sit ABOVE a
/// quoted footer, so the gate proves the composer is drawn on this frame, not
/// that the footer belongs to it. Fails CLOSED — an identity class with no
/// composer signal proves nothing.
fn composer_present(capture: &str, tool: ToolKind) -> bool {
    match tool.adapter().identity {
        IdentitySpec::BorderComposer | IdentitySpec::StyleFooter => true,
        IdentitySpec::RuleFooter => {
            crate::deliver::region::occupancy(capture, tool.input_model())
                != crate::deliver::region::Occupancy::Unreadable
        }
        IdentitySpec::RailStatus | IdentitySpec::BorderText | IdentitySpec::TrailingLabel => {
            crate::deliver::region::composed_ui(capture, tool.adapter().input.composed)
        }
        IdentitySpec::Unmodelled => false,
    }
}

/// Whether `value` is one of the efforts a harness frame may prove.
///
/// Published so the picker's roster fact validates against THIS list rather
/// than a second copy: the vocabulary is closed, so it IS the validation, and
/// a byte cap is only its width.
#[must_use]
pub fn is_effort_word(value: &str) -> bool {
    valid_effort(value)
}

/// Normalize one drawn row: a U+00A0 anywhere becomes a plain space, then
/// ASCII/NBSP whitespace is trimmed from the edges. Only a row carrying an
/// interior NBSP allocates; every other row stays borrowed.
fn normalized_row(line: &str) -> Cow<'_, str> {
    let trimmed = line.trim_matches([' ', '\t', '\r', '\u{a0}']);
    if trimmed.contains('\u{a0}') {
        Cow::Owned(trimmed.replace('\u{a0}', " "))
    } else {
        Cow::Borrowed(trimmed)
    }
}

/// Normalize matching input, dropping blank rows.
fn clean_lines(capture: &str) -> Vec<Cow<'_, str>> {
    capture
        .lines()
        .map(normalized_row)
        .filter(|line| !line.is_empty())
        .collect()
}

/// Every drawn row with blank rows KEPT, for the grammars that anchor on a
/// measured slack to the screen's last ink. `region::rail_box` measures that
/// distance the same way; dropping blanks would renumber it silently.
fn raw_lines(capture: &str) -> Vec<Cow<'_, str>> {
    capture.lines().map(normalized_row).collect()
}

/// The index of the last row carrying ink.
fn last_ink(rows: &[Cow<'_, str>]) -> Option<usize> {
    rows.iter().rposition(|row| !row.is_empty())
}

fn classify_codex(capture: &str) -> HarnessState {
    let Some(frame) = codex_frame(capture) else {
        return HarnessState::Unknown;
    };
    let above = frame.above.as_deref();
    if !codex_placeholder(&frame.prompt)
        || !codex_footer(&frame.footer)
        || above.is_some_and(codex_modal)
    {
        return HarnessState::Unknown;
    }
    if above
        .is_some_and(|line| line.starts_with("• Working (") && line.ends_with("esc to interrupt)"))
    {
        HarnessState::Busy
    } else {
        HarnessState::Idle
    }
}

/// The codex composer's placeholder, which it draws only while the box is empty.
const CODEX_PLACEHOLDER: &str = "› Ask Codex to do anything";

/// Codex's last two inked rows, read as its composer and footer, and the row
/// above the composer.
struct CodexFrame {
    above: Option<String>,
    prompt: String,
    footer: String,
}

/// Read `capture` as a codex frame — the one owner of how its rows are cut.
///
/// Codex's idle starfield paints braille dots over the rows around its
/// composer: a row of dots alone is dropped, and dots on the row above and on
/// the footer read as blanks, so neither the Working line nor the footer parse
/// can be broken by one. The composer row stays RAW, because a dot there may
/// stand in for a placeholder letter ([`codex_placeholder`]).
fn codex_frame(capture: &str) -> Option<CodexFrame> {
    let lines: Vec<Cow<'_, str>> = clean_lines(capture)
        .into_iter()
        .filter(|line| !crate::deliver::region::is_furniture(line))
        .collect();
    let (before, [prompt, footer]) = lines.as_slice().split_last_chunk::<2>()?;
    Some(CodexFrame {
        above: before.last().map(|line| braille_blanked(line)),
        prompt: prompt.to_string(),
        footer: braille_blanked(footer),
    })
}

/// `row` with every braille cell drawn as a blank, edges trimmed.
fn braille_blanked(row: &str) -> String {
    let blanked: String = row
        .chars()
        .map(|ch| {
            if crate::deliver::region::is_braille(ch) {
                ' '
            } else {
                ch
            }
        })
        .collect();
    normalized_row(&blanked).into_owned()
}

/// Whether `row` is the codex placeholder, cell for cell, where a braille cell
/// may stand in for any placeholder cell but the `›` (the letter shimmer), and
/// anything after it is braille and blanks.
fn codex_placeholder(row: &str) -> bool {
    let mut cells = row.chars();
    CODEX_PLACEHOLDER.chars().enumerate().all(|(at, expected)| {
        cells.next().is_some_and(|cell| {
            cell == expected || (at > 0 && crate::deliver::region::is_braille(cell))
        })
    }) && crate::deliver::region::is_furniture(cells.as_str())
}

fn codex_modal(line: &str) -> bool {
    line.contains("Press enter to confirm")
        || line.contains("press enter to confirm")
        || line.contains("esc to cancel")
        || line.contains("trust this")
}

fn classify_claude(capture: &str) -> HarnessState {
    let lines = clean_lines(capture);
    let Some(prompt_index) = lines.iter().rposition(|line| *line == "❯") else {
        return HarnessState::Unknown;
    };
    if !claude_input_frame(&lines, prompt_index) {
        return HarnessState::Unknown;
    }
    let current = lines[..prompt_index]
        .iter()
        .rev()
        .find(|line| !claude_chrome(line));
    match current {
        Some(line) if claude_spinner(line) => HarnessState::Busy,
        Some(line) if claude_done(line) => HarnessState::Idle,
        Some(line) if claude_compacted(line) => HarnessState::Idle,
        None => HarnessState::Idle,
        _ => HarnessState::Unknown,
    }
}

fn claude_input_frame(lines: &[Cow<'_, str>], prompt_index: usize) -> bool {
    let Some(before) = prompt_index
        .checked_sub(1)
        .and_then(|index| lines.get(index))
    else {
        return false;
    };
    let after = &lines[prompt_index + 1..];
    claude_border(before)
        && after.first().is_some_and(|line| claude_border(line))
        && after.iter().all(|line| claude_chrome(line))
        && after.iter().any(|line| line.starts_with('🧠'))
        && after.iter().any(|line| line.starts_with('⏵'))
}

fn claude_spinner(line: &str) -> bool {
    let mut chars = line.chars();
    matches!(chars.next(), Some('✻' | '✽' | '✶' | '✳' | '✢' | '·'))
        && chars.as_str().contains("… (")
}

fn claude_done(line: &str) -> bool {
    line.starts_with("✻ ") && line.contains(" · done ")
}

/// The row Claude Code draws above the box after `/compact`, matched exactly: it
/// reads the frame as ready, never as proof that compaction happened.
fn claude_compacted(line: &str) -> bool {
    line == "⎿  Compacted (ctrl+o to see full summary)"
}

fn claude_chrome(line: &str) -> bool {
    claude_border(line)
        || line.starts_with('🧠')
        || line.starts_with('⏵')
        || line.starts_with('✔')
        || line.starts_with("⎿  Tip:")
}

fn claude_border(line: &str) -> bool {
    !line.is_empty() && line.chars().all(|c| matches!(c, '─' | '━' | '═'))
}

fn codex_footer(line: &str) -> bool {
    let Some((model, effort, path)) = codex_footer_parts(line) else {
        return false;
    };
    // codex 0.156 capitalises the model it draws (`GPT-6-Astra`).
    model
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("gpt-"))
        && valid_effort(effort)
        && !path.is_empty()
}

fn current_claude_identity(capture: &str) -> HarnessIdentity {
    let lines = clean_lines(capture);
    let Some(prompt_index) = lines.iter().rposition(|line| *line == "❯") else {
        return HarnessIdentity::default();
    };
    if !claude_input_frame(&lines, prompt_index) {
        return HarnessIdentity::default();
    }
    lines[prompt_index + 1..]
        .iter()
        .rfind(|line| line.starts_with('🧠'))
        .map_or_else(HarnessIdentity::default, |line| parse_claude_identity(line))
}

fn parse_claude_identity(line: &str) -> HarnessIdentity {
    let Some(rest) = line.strip_prefix("🧠 ") else {
        return HarnessIdentity::default();
    };
    let Some((identity, _chrome)) = rest.split_once("  📁") else {
        return HarnessIdentity::default();
    };
    if let Some(model) = CLAUDE_MODELS
        .iter()
        .find(|candidate| **candidate == identity)
    {
        return HarnessIdentity {
            model: Some((*model).to_owned()),
            effort: None,
        };
    }
    let (model_text, effort) =
        identity
            .rsplit_once(" (")
            .map_or((identity, None), |(model, effort)| {
                (
                    model,
                    effort.strip_suffix(')').filter(|value| valid_effort(value)),
                )
            });
    let model = CLAUDE_MODELS
        .iter()
        .find(|candidate| **candidate == model_text)
        .map(|candidate| (*candidate).to_owned());
    HarnessIdentity {
        model,
        effort: effort.map(str::to_owned),
    }
}

/// The display labels claude's own footer draws. A CLOSED list, so a label that
/// is not here proves NO model: the seat's drift row is never written and the
/// manual choice is never followed. STANDING OBLIGATION, like a toolchain pin —
/// add the label when a new claude model ships, or it stays silently
/// unfollowable.
const CLAUDE_MODELS: [&str; 4] = ["Fable 5.1", "Opus 4.8", "Opus 5 (1M context)", "Opus 5"];

/// Is this the spelling a display-label harness's own footer draws? Because the
/// list above is CLOSED, a `false` here PROVES the value was not read from such
/// a pane — which is how a follow tells a stale observation carried over from
/// another harness from one of its own.
#[must_use]
pub(crate) fn is_claude_model_label(value: &str) -> bool {
    CLAUDE_MODELS.contains(&value)
}

fn current_codex_identity(capture: &str) -> HarnessIdentity {
    let Some(frame) = codex_frame(capture) else {
        return HarnessIdentity::default();
    };
    if !codex_placeholder(&frame.prompt)
        || codex_footer_parts(&frame.footer).is_none()
        || frame.above.as_deref().is_some_and(codex_modal)
    {
        return HarnessIdentity::default();
    }
    parse_codex_identity(&frame.footer)
}

fn codex_footer_parts(line: &str) -> Option<(&str, &str, &str)> {
    let (model_effort, path) = line.split_once(" · ")?;
    let mut words = model_effort.split_whitespace();
    let model = words.next()?;
    let effort = words.next()?;
    (words.next().is_none() && !path.is_empty()).then_some((model, effort, path))
}

/// The model ids codex's `-m` takes. A CLOSED list with the same STANDING
/// OBLIGATION as [`CLAUDE_MODELS`]. codex 0.156 draws a known id by its catalog
/// name (`GPT-6-Sol`), which its API refuses as a model, and codex's
/// observation is replayed into `-m` — so a footer model is matched ignoring
/// ASCII case and read as THIS list's spelling, never as drawn.
const CODEX_MODELS: [&str; 6] = [
    "gpt-5.6-luna",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-6-astra",
    "gpt-6-luna",
    "gpt-6-sol",
];

fn parse_codex_identity(line: &str) -> HarnessIdentity {
    let Some((model_token, effort_token, _path)) = codex_footer_parts(line) else {
        return HarnessIdentity::default();
    };
    let model = CODEX_MODELS
        .iter()
        .find(|id| id.eq_ignore_ascii_case(model_token))
        .map(|id| (*id).to_owned());
    let effort = valid_effort(effort_token).then(|| effort_token.to_owned());
    HarnessIdentity { model, effort }
}

/// A full-width drawn rule: only `─` cells, at least ten of them, the shape
/// `crate::deliver::region::ruled_prompt` fences agy's composer with. The
/// region helper is private, so this mirrors its measured shape.
fn full_rule(row: &str) -> bool {
    row.starts_with('─') && row.chars().all(|cell| cell == '─') && row.chars().count() >= 10
}

/// A rail composer box's bottom edge: the row's first two cells are the
/// corner and underline start of `crate::deliver::region::BoxTable`'s measured
/// tables — opencode's `╹▀`, grok's `╰─`.
fn is_box_edge(row: &str, edge: (char, char)) -> bool {
    let mut cells = row.chars();
    cells.next() == Some(edge.0) && cells.next() == Some(edge.1)
}

/// opencode's measured box: `┃` rails, `╹▀` edge.
const HEAVY_EDGE: (char, char) = ('╹', '▀');

/// grok's measured box: `│` rails, `╰─` edge.
const ROUNDED_EDGE: (char, char) = ('╰', '─');

/// Grok's measured bottom slack: the rounded box's bottom edge sits exactly
/// two rows above the last ink at every captured size, the same `Some(2)`
/// `crate::deliver::region::composed_ui` passes for readiness.
const GROK_BOTTOM_SLACK: usize = 2;

/// opencode's measured upper bound: at the tracked specimen the heavy edge
/// sits seven rows above the last ink (hint, tip and path rows below it).
/// Readiness leaves the heavy rail unanchored because its markers vary;
/// identity reads the same geometry with the measured gap as a ceiling, so a
/// box quoted in scrollback with more rows below it is output, not input.
const OPENCODE_MAX_BOTTOM_SLACK: usize = 7;

/// Muse's composer footer: `model · effort · path · mode`, drawn directly
/// under the composer's closing rule. The rule above the LAST drawn row is
/// the anchor: a footer-shaped transcript row above it is never the current
/// frame. A capture whose composer is absent is out of scope for this
/// grammar — the footer is read only from the bottom pair, so such a capture
/// reads as unobserved rather than as a quoted footer.
fn current_muse_identity(capture: &str) -> HarnessIdentity {
    let lines = clean_lines(capture);
    let Some((footer, before)) = lines.split_last() else {
        return HarnessIdentity::default();
    };
    if !before.last().is_some_and(|line| full_rule(line)) {
        return HarnessIdentity::default();
    }
    let fields: Vec<&str> = footer.split(" · ").collect();
    let [model, effort, _path, mode] = fields.as_slice() else {
        return HarnessIdentity::default();
    };
    if model.is_empty() || mode.is_empty() || !valid_effort(effort) {
        return HarnessIdentity::default();
    }
    HarnessIdentity {
        model: Some((*model).to_owned()),
        effort: Some((*effort).to_owned()),
    }
}

/// opencode's composer status row `mode · model · effort`, drawn directly
/// above the bottom-most `╹▀` heavy edge, which must itself sit within the
/// measured slack of the screen's last ink. The ceiling alone is not the
/// protection: a quoted status+edge pair reads as observed at any slack from
/// zero up to the measured ceiling, and only a pair stranded further than the
/// ceiling is refused. What protects the live frame is that the edge must be
/// the BOTTOM-MOST one and sit at that position, not the bound itself.
fn current_opencode_identity(capture: &str) -> HarnessIdentity {
    let rows = raw_lines(capture);
    let Some(edge) = rows.iter().rposition(|row| is_box_edge(row, HEAVY_EDGE)) else {
        return HarnessIdentity::default();
    };
    let Some(last) = last_ink(&rows) else {
        return HarnessIdentity::default();
    };
    if last.saturating_sub(edge) > OPENCODE_MAX_BOTTOM_SLACK {
        return HarnessIdentity::default();
    }
    let Some(status) = edge.checked_sub(1).and_then(|index| rows.get(index)) else {
        return HarnessIdentity::default();
    };
    // The sidebar shares the status row past the box's own width (measured
    // opencode 1.18.31, width 125 and above with history), so the row is
    // clipped to the edge run BEFORE the split, or the path tail joins the
    // effort field. The run is counted the way `region` clips its interior.
    let run = rows[edge]
        .chars()
        .skip(1)
        .take_while(|&cell| cell == '▀')
        .count();
    let clipped: String = status.chars().take(1 + run).collect();
    let Some(rest) = clipped.strip_prefix('┃') else {
        return HarnessIdentity::default();
    };
    let Some(fields) = crate::deliver::region::heavy_rail_status(rest) else {
        return HarnessIdentity::default();
    };
    let [_, model, effort] = fields.as_slice() else {
        return HarnessIdentity::default();
    };
    if !valid_effort(effort) {
        return HarnessIdentity::default();
    }
    HarnessIdentity {
        model: Some((*model).to_owned()),
        effort: Some((*effort).to_owned()),
    }
}

/// Grok's composer bottom border carries the status text
/// `… · model (effort) · …`. The border is the anchor and it must be the
/// bottom-most `╰─` edge at exactly the measured slack from the last ink: the
/// same bordered row quoted above a live frame proves nothing, and a border
/// stranded with MORE than the measured slack of rows below it proves
/// nothing. At exactly the measured slack it is indistinguishable from the
/// live frame and is read.
fn current_grok_identity(capture: &str) -> HarnessIdentity {
    let rows = raw_lines(capture);
    let Some(border) = rows.iter().rposition(|row| is_box_edge(row, ROUNDED_EDGE)) else {
        return HarnessIdentity::default();
    };
    let Some(last) = last_ink(&rows) else {
        return HarnessIdentity::default();
    };
    if border + GROK_BOTTOM_SLACK != last {
        return HarnessIdentity::default();
    }
    let inner = rows[border].trim_start_matches('╰').trim_end_matches('╯');
    let mut found: Option<(&str, &str)> = None;
    for segment in inner.split(" · ") {
        let Some((model, effort)) = segment.rsplit_once(" (") else {
            continue;
        };
        let Some(effort) = effort.strip_suffix(')') else {
            continue;
        };
        if model.is_empty() || !valid_effort(effort) {
            continue;
        }
        if found.is_some() {
            // Two parenthesized efforts on one border: ambiguous, unobserved.
            return HarnessIdentity::default();
        }
        found = Some((model, effort));
    }
    found.map_or_else(HarnessIdentity::default, |(model, effort)| {
        HarnessIdentity {
            model: Some(model.to_owned()),
            effort: Some(effort.to_owned()),
        }
    })
}

/// Agy's model label: the LAST drawn row's trailing `model · effort`, set
/// apart from a hint on its left by a run of spaces, with the composer's own
/// full-width rule within the three rows above it. The folder-trust modal
/// draws the same label without any rule and is unobserved, never the live
/// frame. Refused is only an interior separator in the model (after the
/// two-space split); a sentence that merely ENDS in ` · <effort>` with no such
/// separator still reads as observed. A
/// wrapped label continuation cannot arrive as its own row: the watchdog
/// captures with `capture-pane -p -J`, which joins wrapped rows
/// (tmux.rs:993), so the grammar is never handed one.
fn current_agy_identity(capture: &str) -> HarnessIdentity {
    let rows = raw_lines(capture);
    let Some(last) = last_ink(&rows) else {
        return HarnessIdentity::default();
    };
    if !rows[last.saturating_sub(3)..last]
        .iter()
        .any(|row| full_rule(row))
    {
        return HarnessIdentity::default();
    }
    let Some((left, effort)) = rows[last].rsplit_once(" · ") else {
        return HarnessIdentity::default();
    };
    if !valid_effort(effort) {
        return HarnessIdentity::default();
    }
    // The hint on the left is separated by a run of spaces; a label-only row
    // has none. Either way the separator must not survive into the model.
    let model = left
        .rsplit_once("  ")
        .map_or(left, |(_, model)| model)
        .trim();
    if model.is_empty() || model.contains(" · ") {
        return HarnessIdentity::default();
    }
    HarnessIdentity {
        model: Some(model.to_owned()),
        effort: Some(effort.to_owned()),
    }
}

fn valid_effort(value: &str) -> bool {
    matches!(value, "low" | "medium" | "high" | "xhigh" | "max" | "ultra")
}

#[cfg(test)]
mod tests {
    use super::{
        HarnessIdentity, HarnessState, IdleCarry, classify, claude_input_frame, clean_lines,
        current_identity, decode_idle, encode_idle, has_human_draft, is_effort_word,
        observed_from_option, observed_identity,
    };
    use crate::tool::ToolKind;

    #[test]
    fn a_live_codex_working_frame_is_busy() {
        let capture = include_str!("../tests/fixtures/harness-state/codex-busy-280x40.txt");
        assert_eq!(classify(capture, ToolKind::Codex), HarnessState::Busy);
    }

    #[test]
    fn a_live_codex_empty_input_frame_is_idle() {
        let capture = include_str!("../tests/fixtures/harness-state/codex-idle-112x40.txt");
        assert_eq!(classify(capture, ToolKind::Codex), HarnessState::Idle);
    }

    #[test]
    fn a_live_claude_spinner_frame_is_busy() {
        let capture = include_str!("../tests/fixtures/harness-state/claude-busy-280x40.txt");
        assert_eq!(classify(capture, ToolKind::Claude), HarnessState::Busy);
    }

    #[test]
    fn a_live_claude_done_frame_is_idle() {
        let capture = include_str!("../tests/fixtures/harness-state/claude-idle-167x40.txt");
        assert_eq!(classify(capture, ToolKind::Claude), HarnessState::Idle);
    }

    #[test]
    fn a_second_live_claude_spinner_grammar_is_busy() {
        let capture = include_str!("../tests/fixtures/harness-state/claude-busy-167x40.txt");
        assert_eq!(classify(capture, ToolKind::Claude), HarnessState::Busy);
    }

    /// Live capture 2026-09-14 from pane `%0` (session `aedev`, Claude Code
    /// 2.1.270) with `capture-pane -p -J -S -40 -E -`; only the transcript
    /// above the frame is redacted. The tip line carries the live bytes
    /// `e2 8e bf 20 c2 a0` (`⎿`, space, NBSP) that the frozen ASCII-space
    /// fixtures cannot, and shadows the spinner line above it.
    #[test]
    fn a_live_claude_busy_frame_with_an_nbsp_tip_is_busy() {
        let capture =
            include_str!("../tests/fixtures/harness-state/claude-busy-nbsp-tip-101x41.txt");
        assert_eq!(classify(capture, ToolKind::Claude), HarnessState::Busy);
    }

    #[test]
    fn clean_lines_normalizes_an_interior_nbsp_for_the_whole_line() {
        let lines = clean_lines("  ⎿\u{a0}Tip: use\u{a0}/btw  \nplain line\n");
        assert_eq!(lines[0], "⎿ Tip: use /btw");
        assert_eq!(lines[1], "plain line");
    }

    #[test]
    fn a_resumed_live_claude_done_frame_is_idle() {
        let capture =
            include_str!("../tests/fixtures/harness-state/claude-resumed-idle-149x37.txt");
        assert_eq!(classify(capture, ToolKind::Claude), HarnessState::Idle);
    }

    const COMPACTED: &str =
        include_str!("../tests/fixtures/harness-state/claude-compacted-2.1.280.txt");
    const ROW: &str = "⎿  Compacted (ctrl+o to see full summary)";

    /// `frame` with its one exact `needle` replaced once.
    fn swap_once(frame: &str, needle: &str, with: &str) -> String {
        assert_eq!(frame.matches(needle).count(), 1, "one needle: {needle}");
        frame.replacen(needle, with, 1)
    }

    /// SYNTHETIC: the frame's one manual `⏸` footer row, which
    /// `claude_input_frame` rejects, swapped once for this module's bypass footer.
    fn bypass(frame: &str) -> String {
        let manual: Vec<&str> = frame
            .lines()
            .filter(|row| row.trim_start().starts_with('⏸'))
            .collect();
        assert_eq!(manual.len(), 1, "exactly one manual footer row");
        swap_once(frame, manual[0], "  ⏵⏵ bypass permissions on")
    }

    /// The premise: a valid input frame around the bare prompt.
    fn gate(frame: &str) -> bool {
        let lines = clean_lines(frame);
        let prompt = lines.iter().rposition(|line| *line == "❯");
        prompt.is_some_and(|at| claude_input_frame(&lines, at))
    }

    #[test]
    fn a_compacted_frame_with_a_passing_footer_is_idle() {
        let capture = bypass(COMPACTED);
        assert!(gate(&capture), "premise: a valid input frame");
        assert_eq!(classify(&capture, ToolKind::Claude), HarnessState::Idle);
    }

    #[test]
    fn only_an_exact_nearest_compacted_row_is_idle() {
        let capture = bypass(COMPACTED);
        for (nearest, want) in [
            (format!("{ROW}\n✶ Thinking… (2s)"), HarnessState::Busy),
            (format!("{ROW}\nan unfamiliar row"), HarnessState::Unknown),
            (ROW.replace("full", "the"), HarnessState::Unknown),
            (format!("Note: {ROW}"), HarnessState::Unknown),
            (format!("{ROW} again"), HarnessState::Unknown),
        ] {
            let frame = swap_once(&capture, ROW, &nearest);
            assert!(gate(&frame), "premise: {nearest}");
            assert_eq!(classify(&frame, ToolKind::Claude), want, "{nearest}");
        }
    }

    #[test]
    fn a_compacting_frame_is_not_idle() {
        let capture = bypass(include_str!(
            "../tests/fixtures/harness-state/claude-compacting-2.1.280.txt"
        ));
        assert!(gate(&capture), "premise: a valid input frame");
        assert_eq!(classify(&capture, ToolKind::Claude), HarnessState::Unknown);
    }

    #[test]
    fn a_dispatch_left_in_or_queued_above_the_box_is_not_idle() {
        for landed in [
            include_str!("../tests/fixtures/claude-composer/claude-compact-staged-2.1.280.txt"),
            include_str!("../tests/fixtures/claude-composer/claude-compact-queued-2.1.280.txt"),
        ] {
            let frame = bypass(landed);
            let row = frame
                .lines()
                .rfind(|line| line.starts_with('❯'))
                .expect("a box row");
            assert!(gate(&swap_once(&frame, row, "❯")), "premise: {row}");
            assert_eq!(classify(&frame, ToolKind::Claude), HarnessState::Unknown);
            assert!(has_human_draft(&frame, ToolKind::Claude), "{row}");
        }
    }

    #[test]
    fn a_compacted_frame_in_manual_mode_stays_unknown() {
        let lines = clean_lines(COMPACTED);
        let prompt = lines.iter().rposition(|line| *line == "❯");
        let nearest = &lines[prompt.expect("a bare prompt") - 2];
        assert!(super::claude_compacted(nearest), "premise: {nearest}");
        assert!(!gate(COMPACTED), "the manual footer fails the frame");
        assert_eq!(classify(COMPACTED, ToolKind::Claude), HarnessState::Unknown);
    }

    #[test]
    fn the_compacted_row_changes_neither_identity_nor_draft() {
        let compacted = bypass(COMPACTED);
        let done = swap_once(&compacted, ROW, "✻ Worked for 2s · done 9:10 PM");
        let identity = current_identity(&compacted, ToolKind::Claude);
        assert_eq!(identity.effort.as_deref(), Some("xhigh"), "premise: read");
        assert_eq!(identity, current_identity(&done, ToolKind::Claude));
        let draft = has_human_draft(&compacted, ToolKind::Claude);
        assert!(!draft, "premise: an empty box");
        assert_eq!(draft, has_human_draft(&done, ToolKind::Claude));
    }

    #[test]
    fn a_fresh_claude_empty_box_is_idle_but_a_prompt_alone_is_not() {
        let border = "────────────────────────────────────────────────────────────────";
        let fresh = format!(
            "{border}\n❯\n{border}\n  🧠 Opus 5 (xhigh)  📁 ae\n  ⏵⏵ bypass permissions on\n"
        );
        assert_eq!(classify(&fresh, ToolKind::Claude), HarnessState::Idle);

        let ambiguous = format!(
            "unfamiliar current status\n{border}\n❯\n{border}\n  🧠 Opus 5 (xhigh)  📁 ae\n"
        );
        assert_eq!(
            classify(&ambiguous, ToolKind::Claude),
            HarnessState::Unknown,
            "prompt-visible without a known current status is not enough"
        );
    }

    #[test]
    fn witness_a_clipped_prompt_only_frames_are_unknown() {
        let live = include_str!("../tests/fixtures/harness-state/claude-idle-167x40.txt");
        assert_eq!(classify(live, ToolKind::Claude), HarnessState::Idle);

        assert_eq!(
            classify("❯\n", ToolKind::Claude),
            HarnessState::Unknown,
            "a bare prompt has no positive current-frame evidence"
        );

        let border = "────────────────────────────────────────────────────────────────";
        let clipped = format!("{border}\n❯\n{border}\n");
        assert_eq!(
            classify(&clipped, ToolKind::Claude),
            HarnessState::Unknown,
            "a clipped input box has no positive current-frame evidence"
        );
    }

    #[test]
    fn an_old_quoted_busy_line_does_not_override_the_current_codex_idle_frame() {
        let capture =
            include_str!("../tests/fixtures/harness-state/codex-idle-old-busy-280x40.txt");
        assert_eq!(classify(capture, ToolKind::Codex), HarnessState::Idle);
    }

    #[test]
    fn a_short_live_codex_idle_frame_is_idle() {
        let capture = include_str!("../tests/fixtures/harness-state/codex-idle-112x20.txt");
        assert_eq!(classify(capture, ToolKind::Codex), HarnessState::Idle);
    }

    #[test]
    fn a_narrow_wrapped_live_codex_idle_frame_is_idle() {
        let capture = include_str!("../tests/fixtures/harness-state/codex-idle-wrapped-112x40.txt");
        assert_eq!(classify(capture, ToolKind::Codex), HarnessState::Idle);
    }

    #[test]
    fn current_identity_reads_real_claude_and_codex_footer_anchors() {
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/harness-state/claude-busy-280x40.txt"),
                ToolKind::Claude
            ),
            HarnessIdentity {
                model: Some("Opus 5".to_owned()),
                effort: Some("xhigh".to_owned())
            }
        );
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/harness-state/codex-idle-112x40.txt"),
                ToolKind::Codex
            ),
            HarnessIdentity {
                model: Some("gpt-6-astra".to_owned()),
                effort: Some("xhigh".to_owned())
            }
        );
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/runtime-identity/claude-opus-1m.txt"),
                ToolKind::Claude
            ),
            HarnessIdentity {
                model: Some("Opus 5 (1M context)".to_owned()),
                effort: Some("xhigh".to_owned())
            }
        );
    }

    #[test]
    fn current_identity_keeps_unknown_fields_independent() {
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/runtime-identity/codex-unknown-model.txt"),
                ToolKind::Codex
            ),
            HarnessIdentity {
                model: None,
                effort: Some("high".to_owned())
            }
        );
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/runtime-identity/claude-unknown-model.txt"),
                ToolKind::Claude
            ),
            HarnessIdentity {
                model: None,
                effort: Some("medium".to_owned())
            }
        );
        assert_eq!(
            current_identity(
                include_str!(
                    "../tests/fixtures/runtime-identity/codex-known-model-unknown-effort.txt"
                ),
                ToolKind::Codex
            ),
            HarnessIdentity {
                model: Some("gpt-5.6-sol".to_owned()),
                effort: None
            }
        );
        assert_eq!(
            current_identity(
                include_str!(
                    "../tests/fixtures/runtime-identity/claude-known-model-unknown-effort.txt"
                ),
                ToolKind::Claude
            ),
            HarnessIdentity {
                model: Some("Opus 5".to_owned()),
                effort: None
            }
        );
    }

    #[test]
    fn current_identity_rejects_modal_and_clipped_frames() {
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/harness-state/codex-modal-112x40.txt"),
                ToolKind::Codex
            ),
            HarnessIdentity::default()
        );
        assert_eq!(
            current_identity("❯\n", ToolKind::Claude),
            HarnessIdentity::default()
        );
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/runtime-identity/codex-hostile-footer.txt"),
                ToolKind::Codex
            ),
            HarnessIdentity::default()
        );
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/runtime-identity/codex-control-token.txt"),
                ToolKind::Codex
            ),
            HarnessIdentity {
                model: None,
                effort: Some("high".to_owned())
            }
        );
        let two_lines_above =
            include_str!("../tests/fixtures/runtime-identity/codex-modal-two-lines-above.txt");
        assert_eq!(
            classify(two_lines_above, ToolKind::Codex),
            HarnessState::Idle
        );
        assert_eq!(
            current_identity(two_lines_above, ToolKind::Codex),
            HarnessIdentity {
                model: Some("gpt-6-astra".to_owned()),
                effort: Some("xhigh".to_owned())
            }
        );
    }

    #[test]
    fn current_identity_bounded_model_table_covers_configured_fleet() {
        for (model, expected) in [
            ("gpt-5.6-luna", "gpt-5.6-luna"),
            ("gpt-5.6-sol", "gpt-5.6-sol"),
            ("gpt-5.6-terra", "gpt-5.6-terra"),
            ("gpt-6-astra", "gpt-6-astra"),
        ] {
            let capture = format!("› Ask Codex to do anything\n\n  {model} high · ~/ae\n");
            assert_eq!(
                current_identity(&capture, ToolKind::Codex).model.as_deref(),
                Some(expected)
            );
        }
        for model in ["Fable 5.1", "Opus 4.8", "Opus 5"] {
            let capture =
                format!("────\n❯\n────\n  🧠 {model} (high)  📁 ae\n  ⏵⏵ bypass permissions on\n");
            assert_eq!(
                current_identity(&capture, ToolKind::Claude)
                    .model
                    .as_deref(),
                Some(model)
            );
        }
        for model in ["Opus 50", "Opus 5 (unrecognised context)"] {
            let capture =
                format!("────\n❯\n────\n  🧠 {model} (high)  📁 ae\n  ⏵⏵ bypass permissions on\n");
            assert_eq!(
                current_identity(&capture, ToolKind::Claude),
                HarnessIdentity {
                    model: None,
                    effort: Some("high".to_owned())
                }
            );
        }
        let later_path = "────\n❯\n────\n  🧠 Opus 5  📁 ae (high)\n  ⏵⏵ bypass permissions on\n";
        assert_eq!(
            current_identity(later_path, ToolKind::Claude),
            HarnessIdentity {
                model: Some("Opus 5".to_owned()),
                effort: None
            }
        );
        let model_without_effort =
            "────\n❯\n────\n  🧠 Opus 5 (1M context)  📁 ae\n  ⏵⏵ bypass permissions on\n";
        assert_eq!(
            current_identity(model_without_effort, ToolKind::Claude),
            HarnessIdentity {
                model: Some("Opus 5 (1M context)".to_owned()),
                effort: None
            }
        );
    }

    #[test]
    fn current_identity_tracks_changed_effort_and_ignores_historical_footer() {
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/runtime-identity/codex-sol-medium.txt"),
                ToolKind::Codex
            ),
            HarnessIdentity {
                model: Some("gpt-5.6-sol".to_owned()),
                effort: Some("medium".to_owned())
            }
        );
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/runtime-identity/codex-historical-conflict.txt"),
                ToolKind::Codex
            ),
            HarnessIdentity {
                model: Some("gpt-6-astra".to_owned()),
                effort: Some("low".to_owned())
            }
        );
    }

    #[test]
    fn current_identity_rejects_draft_and_unsupported_frames() {
        assert_eq!(
            current_identity(
                "[redacted]\n› keep this unsent\n\n  gpt-5.6-sol xhigh · ~/ae\n",
                ToolKind::Codex
            ),
            HarnessIdentity::default()
        );
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/harness-state/codex-idle-112x40.txt"),
                ToolKind::Grok
            ),
            HarnessIdentity::default()
        );
    }

    #[test]
    fn a_muse_composer_footer_proves_model_and_effort() {
        let expected = HarnessIdentity {
            model: Some("muse-spark-1.3".to_owned()),
            effort: Some("max".to_owned()),
        };
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/runtime-identity/muse-idle-plain-80x24.txt"),
                ToolKind::Muse
            ),
            expected
        );
        // A staged turn keeps the same footer, with a hint after the mode.
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/runtime-identity/muse-occupied-plain-80x24.txt"),
                ToolKind::Muse
            ),
            expected
        );
    }

    #[test]
    fn a_muse_footer_without_its_composer_rule_is_unobserved() {
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/runtime-identity/muse-transcript-only.txt"),
                ToolKind::Muse
            ),
            HarnessIdentity::default()
        );
    }

    #[test]
    fn a_blank_muse_frame_is_unobserved() {
        assert_eq!(
            current_identity("", ToolKind::Muse),
            HarnessIdentity::default()
        );
        assert_eq!(
            current_identity("\n \n", ToolKind::Muse),
            HarnessIdentity::default()
        );
    }

    #[test]
    fn an_opencode_composer_status_row_proves_model_and_effort() {
        assert_eq!(
            current_identity(
                include_str!(
                    "../tests/fixtures/runtime-identity/opencode-composed-plain-80x24.txt"
                ),
                ToolKind::OpenCode
            ),
            HarnessIdentity {
                model: Some("DeepSeek V4.1 Flash OpenRouter".to_owned()),
                effort: Some("max".to_owned()),
            }
        );
    }

    #[test]
    fn an_opencode_seat_with_history_observes_its_model_at_both_widths() {
        const HISTORY_EMPTY: &str =
            include_str!("../tests/fixtures/opencode-composer/opencode-history-empty-80x24.txt");
        const HISTORY_EMPTY_WIDE: &str =
            include_str!("../tests/fixtures/opencode-composer/opencode-history-empty-200x50.txt");
        const HISTORY_DRAFT: &str =
            include_str!("../tests/fixtures/opencode-composer/opencode-history-draft-80x24.txt");
        let observed = HarnessIdentity {
            model: Some("DeepSeek V4.1 Flash OpenRouter".to_owned()),
            effort: Some("max".to_owned()),
        };
        assert_eq!(
            observed_identity(HISTORY_EMPTY, ToolKind::OpenCode),
            observed
        );
        assert_eq!(
            observed_identity(HISTORY_EMPTY_WIDE, ToolKind::OpenCode),
            observed,
            "the sidebar tail past the edge run never reaches the split"
        );
        assert_eq!(
            observed_identity(HISTORY_DRAFT, ToolKind::OpenCode),
            HarnessIdentity::default(),
            "a draft refuses the gate, so the status row is not read"
        );
    }

    #[test]
    fn an_unmeasured_status_shape_is_unobserved() {
        // The picker cell keeps the strict policy — exactly three fields
        // plus the closed effort vocabulary — while delivery proves
        // structure alone (pinned in region).
        let frame = |status: &str| format!("  ┃\n  ┃\n  ┃  {status}\n  ╹▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀\n");
        assert_eq!(
            observed_identity(&frame("Build · m · reasoning"), ToolKind::OpenCode),
            HarnessIdentity::default()
        );
        assert_eq!(
            observed_identity(&frame("Build · m"), ToolKind::OpenCode),
            HarnessIdentity::default()
        );
    }

    #[test]
    fn an_opencode_status_row_without_the_composer_edge_is_unobserved() {
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/runtime-identity/opencode-transcript-only.txt"),
                ToolKind::OpenCode
            ),
            HarnessIdentity::default()
        );
    }

    #[test]
    fn a_blank_opencode_frame_is_unobserved() {
        assert_eq!(
            current_identity("", ToolKind::OpenCode),
            HarnessIdentity::default()
        );
        assert_eq!(
            current_identity("\n\n", ToolKind::OpenCode),
            HarnessIdentity::default()
        );
    }

    #[test]
    fn a_grok_composer_border_proves_model_and_effort() {
        let expected = HarnessIdentity {
            model: Some("Grok 4.6".to_owned()),
            effort: Some("high".to_owned()),
        };
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/grok-composer/grok-composed-frame-80x24.txt"),
                ToolKind::Grok
            ),
            expected
        );
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/grok-composer/grok-composed-frame.txt"),
                ToolKind::Grok
            ),
            expected
        );
    }

    #[test]
    fn a_grok_border_echo_above_the_live_frame_is_unobserved() {
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/runtime-identity/grok-transcript-only.txt"),
                ToolKind::Grok
            ),
            HarnessIdentity::default()
        );
    }

    #[test]
    fn a_blank_grok_frame_is_unobserved() {
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/grok-composer/grok-boot-frame.txt"),
                ToolKind::Grok
            ),
            HarnessIdentity::default()
        );
        assert_eq!(
            current_identity("", ToolKind::Grok),
            HarnessIdentity::default()
        );
    }

    #[test]
    fn an_agy_model_label_proves_model_and_effort() {
        let expected = HarnessIdentity {
            model: Some("Gemini 3.8 Flash".to_owned()),
            effort: Some("high".to_owned()),
        };
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/agy-composer/agy-composed-frame.txt"),
                ToolKind::Agy
            ),
            expected
        );
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/agy-composer/agy-draft-frame-80x24.txt"),
                ToolKind::Agy
            ),
            expected
        );
    }

    #[test]
    fn an_agy_trust_modal_is_not_the_live_composer() {
        // The modal carries the label with no rule anywhere; the composer's
        // rule proximity is the anchor, so the modal is unobserved.
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/agy-composer/agy-trust-modal-frame.txt"),
                ToolKind::Agy
            ),
            HarnessIdentity::default()
        );
    }

    #[test]
    fn an_agy_label_echo_without_a_live_footer_is_unobserved() {
        let echoed = include_str!("../tests/fixtures/runtime-identity/agy-transcript-only.txt");
        assert_eq!(
            current_identity(echoed, ToolKind::Agy),
            HarnessIdentity::default()
        );
        // The anchor is the rule above the LAST row, not the row order: with
        // the hint row deleted the label is not last and still unobserved.
        let without_hint = echoed
            .lines()
            .filter(|line| !line.contains("? for shortcuts"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            current_identity(&without_hint, ToolKind::Agy),
            HarnessIdentity::default()
        );
        // A lone label as the LAST row with no rule above it is not the live
        // composer: the rule proximity is the anchor that refuses it.
        assert_eq!(
            current_identity("Gemini 3.8 Flash · high\n", ToolKind::Agy),
            HarnessIdentity::default()
        );
        // A sentence that happens to end in ` · high` is not a label.
        let sentence = format!("{}\nbudget · plan · quota is · high\n", "─".repeat(40));
        assert_eq!(
            current_identity(&sentence, ToolKind::Agy),
            HarnessIdentity::default()
        );
    }

    #[test]
    fn a_boot_agy_frame_is_unobserved() {
        assert_eq!(
            current_identity(
                include_str!("../tests/fixtures/agy-composer/agy-boot-frame.txt"),
                ToolKind::Agy
            ),
            HarnessIdentity::default()
        );
    }

    #[test]
    fn a_modal_over_an_input_box_is_unknown() {
        let capture = include_str!("../tests/fixtures/harness-state/codex-modal-112x40.txt");
        assert_eq!(classify(capture, ToolKind::Codex), HarnessState::Unknown);
    }

    #[test]
    fn unsupported_harnesses_are_unknown_even_with_a_familiar_prompt() {
        let capture = include_str!("../tests/fixtures/harness-state/codex-idle-112x40.txt");
        assert_eq!(classify(capture, ToolKind::Grok), HarnessState::Unknown);
    }

    #[test]
    fn only_current_modeled_input_text_is_a_human_draft() {
        let draft = "[redacted]\n› keep this unsent\n\n  gpt-5.6-sol xhigh · ~/ae\n";
        assert!(has_human_draft(draft, ToolKind::Codex));
        let border = "────────────────────────────────────────────────────────────────";
        let claude = format!(
            "{border}\n❯ keep this unsent\n{border}\n  🧠 Opus 5 (xhigh)  📁 ae\n  ⏵⏵ bypass permissions on\n"
        );
        assert!(has_human_draft(&claude, ToolKind::Claude));
        assert!(!has_human_draft(
            include_str!("../tests/fixtures/harness-state/codex-busy-280x40.txt"),
            ToolKind::Codex
        ));
        assert!(!has_human_draft("❯ draft", ToolKind::Grok));
    }

    /// Codex's idle starfield, as measured (provenance beside the fixture).
    const CODEX_STARFIELD: &str =
        include_str!("../tests/fixtures/harness-state/codex-starfield-216x6.txt");

    #[test]
    fn a_codex_starfield_is_furniture_to_the_frame_grammar() {
        assert_eq!(
            classify(CODEX_STARFIELD, ToolKind::Codex),
            HarnessState::Idle
        );
        assert!(!has_human_draft(CODEX_STARFIELD, ToolKind::Codex));
        let busy = CODEX_STARFIELD.replace(
            "  Worked for 14m 51s · done 12:00 PM",
            "• Working (9s • esc to interrupt)  ⠁ ⠈",
        );
        assert_eq!(classify(&busy, ToolKind::Codex), HarnessState::Busy);
        // A dot where a footer space was, and dots trailing it.
        let footer = CODEX_STARFIELD.replace("gpt-6-astra xhigh ·", "gpt-6-astra⠁xhigh ·  ⠂");
        assert_eq!(classify(&footer, ToolKind::Codex), HarnessState::Idle);
        let frame = |composer: &str| {
            format!("• ok\n ⠈   ⠁\n{composer}\n   ⠐  ⠄\n  gpt-6-astra xhigh · ~/ae\n")
        };
        // The letter shimmer swaps placeholder cells for dots.
        let shimmer = frame("›⠁Ask Codex to do a⠈yth⢀ng  ⠂");
        assert_eq!(classify(&shimmer, ToolKind::Codex), HarnessState::Idle);
        assert!(!has_human_draft(&shimmer, ToolKind::Codex));
        let draft = frame("› fix the bug ⠁  ⠈");
        assert!(has_human_draft(&draft, ToolKind::Codex));
        assert_eq!(classify(&draft, ToolKind::Codex), HarnessState::Unknown);
        assert_eq!(
            current_identity(&draft, ToolKind::Codex),
            HarnessIdentity::default()
        );
        assert_eq!(
            current_identity(&shimmer, ToolKind::Codex).model.as_deref(),
            Some("gpt-6-astra")
        );
        // No draft is read over a footer that is not codex's.
        let foreign = draft.replace("gpt-6-astra xhigh", "claude-x xhigh");
        assert!(!has_human_draft(&foreign, ToolKind::Codex));
        // A dot never stands in for the ornament, and a letter never for a dot.
        let ornament = frame("⠁ Ask Codex to do anything");
        assert_eq!(classify(&ornament, ToolKind::Codex), HarnessState::Unknown);
        let respelled = frame("› Ask Codex to do anythinG");
        assert_eq!(classify(&respelled, ToolKind::Codex), HarnessState::Unknown);
        assert!(has_human_draft(&respelled, ToolKind::Codex));
        // Dots alone after the ornament are no draft, and no placeholder
        // either: the frame stays unrecognised rather than guessed idle.
        let dots = frame("›⠁  ⠈ ⠂");
        assert!(!has_human_draft(&dots, ToolKind::Codex));
        assert_eq!(classify(&dots, ToolKind::Codex), HarnessState::Unknown);
        // An empty box is no draft either (it read as one before the dots).
        assert!(!has_human_draft(&frame("›"), ToolKind::Codex));
        let short = frame("›⠁Ask Codex");
        assert_eq!(classify(&short, ToolKind::Codex), HarnessState::Unknown);
        assert!(has_human_draft(&short, ToolKind::Codex));
    }

    #[test]
    fn codex_0_156_capitalises_its_footer_model_and_still_classifies() {
        for capture in [
            include_str!("../tests/fixtures/harness-state/codex-idle-0.155.1-200x40.txt"),
            include_str!("../tests/fixtures/harness-state/codex-idle-0.156.1-200x40.txt"),
        ] {
            assert_eq!(classify(capture, ToolKind::Codex), HarnessState::Idle);
            let busy = capture.replace("  12:50", "• Working (3s • esc to interrupt)");
            let busy = busy.replace("  done 12:53 PM", "• Working (3s • esc to interrupt)");
            assert_eq!(classify(&busy, ToolKind::Codex), HarnessState::Busy);
        }
        let other = "› Ask Codex to do anything\n\n  claude-x high · ~/ae\n";
        assert_eq!(classify(other, ToolKind::Codex), HarnessState::Unknown);
        let effort = "› Ask Codex to do anything\n\n  GPT-6-Astra turbo · ~/ae\n";
        assert_eq!(classify(effort, ToolKind::Codex), HarnessState::Unknown);
    }

    #[test]
    fn a_codex_display_name_reads_as_the_id_its_flag_takes() {
        // Measured on 0.156.1: `-m <id>` draws the catalog name, and the API
        // refuses that name as a model (400 on the first turn).
        let footer = |drawn: &str| {
            format!(
                "› Ask Codex to do anything\n\n  {drawn} xhigh · ~/projects/clemens33/ae · main\n"
            )
        };
        for (drawn, id) in [
            ("GPT-6-Sol", "gpt-6-sol"),
            ("GPT-6-Luna", "gpt-6-luna"),
            ("GPT-6-Astra", "gpt-6-astra"),
            ("GPT-5.6-Sol", "gpt-5.6-sol"),
            ("GPT-5.6-Luna", "gpt-5.6-luna"),
            ("GPT-5.6-Terra", "gpt-5.6-terra"),
            ("GPT-6-SOL", "gpt-6-sol"),
            ("gpt-6-luna", "gpt-6-luna"),
        ] {
            assert_eq!(
                current_identity(&footer(drawn), ToolKind::Codex),
                HarnessIdentity {
                    model: Some(id.to_owned()),
                    effort: Some("xhigh".to_owned())
                },
                "{drawn}"
            );
        }
        // An id codex does not know is drawn raw; near misses are no model.
        for drawn in ["gpt-6-nosuchmodel", "GPT-6-Astral", "GPT-6", "GPT‐6‐Sol"] {
            assert_eq!(
                current_identity(&footer(drawn), ToolKind::Codex).model,
                None,
                "{drawn}"
            );
        }
        for (capture, drawn) in [
            (
                include_str!("../tests/fixtures/harness-state/codex-idle-0.155.1-200x40.txt"),
                "gpt-6-astra",
            ),
            (
                include_str!("../tests/fixtures/harness-state/codex-idle-0.156.1-200x40.txt"),
                "GPT-6-Astra",
            ),
        ] {
            assert!(capture.contains(&format!("  {drawn} low · ")), "{drawn}");
            let identity = current_identity(capture, ToolKind::Codex);
            assert_eq!(identity.model.as_deref(), Some("gpt-6-astra"), "{drawn}");
            // The profile's lowercase pin is satisfied: no drift row is written.
            assert_eq!(
                crate::model_drift::decide(
                    &identity,
                    Some("gpt-6-astra"),
                    crate::tool::PinMatch::Exact
                ),
                crate::model_drift::Decision::Retire,
                "{drawn}"
            );
        }
    }

    #[test]
    fn a_six_pointed_star_is_claudes_spinner_too() {
        // Measured rows, Claude Code 2.1.280 (ctxprobe compact-002..004).
        for row in [
            "✶ Crystallizing… (2s · thinking with xhigh effort)",
            "✶ Crystallizing… (4s · thinking with xhigh effort)",
            "✶ Crystallizing… (6s · ↓ 113 tokens · thought for 4s)",
        ] {
            assert!(super::claude_spinner(row), "{row}");
            let frame = format!(
                "{row}\n\n────\n❯\n────\n  🧠 Opus 5.5 (xhigh)  📁 seat\n  ⏵⏵ bypass permissions on\n"
            );
            assert_eq!(classify(&frame, ToolKind::Claude), HarnessState::Busy);
        }
        assert!(!super::claude_spinner("✶ Crystallizing…"));
    }

    #[test]
    fn a_historical_prompt_above_a_modal_is_not_a_current_human_draft() {
        let codex = "› [redacted submitted turn]\nAllow command? Press enter to confirm\n\n  gpt-6-astra xhigh · ~/ae\n";
        assert!(!has_human_draft(codex, ToolKind::Codex));

        let claude = "❯ [redacted submitted turn]\nAllow command? Press enter to confirm\n";
        assert!(!has_human_draft(claude, ToolKind::Claude));
    }

    #[test]
    fn observed_option_is_total_and_idle_carry_round_trips() {
        let carry = IdleCarry {
            since_epoch: 42,
            nudges: 2,
            undelivered: 1,
            identity: 99,
            declaration: Some(7),
        };
        let encoded = encode_idle(carry);
        assert_eq!(decode_idle(&encoded), Some(carry));
        assert_eq!(observed_from_option(&encoded), HarnessState::Idle);
        assert_eq!(
            decode_idle("idle:42:2:1:99"),
            Some(IdleCarry {
                declaration: None,
                ..carry
            }),
            "the pre-declaration-marker carry stays readable"
        );
        assert_eq!(observed_from_option("busy"), HarnessState::Busy);
        for malformed in [
            "",
            "idle:x:2:1:99",
            "idle:42:2:1",
            "idle:42:2:1:99:x",
            "busy:junk",
            "modal",
        ] {
            assert_eq!(
                observed_from_option(malformed),
                HarnessState::Unknown,
                "{malformed}"
            );
            assert_eq!(decode_idle(malformed), None, "{malformed}");
        }
    }

    #[test]
    fn every_intended_valid_observed_fuzz_seed_decodes_without_normalization() {
        for (name, seed) in [
            (
                "current-idle",
                include_str!("../fuzz/seeds/harness_observed/current-idle"),
            ),
            (
                "current-unknown",
                include_str!("../fuzz/seeds/harness_observed/current-unknown"),
            ),
            (
                "legacy-idle",
                include_str!("../fuzz/seeds/harness_observed/legacy-idle"),
            ),
        ] {
            assert!(decode_idle(seed).is_some(), "{name}: {seed:?}");
        }
    }

    /// The residual the module doc names, closed for the four bottom-anchored
    /// grammars: a frame that ENDS at the tool's live geometry while the
    /// tool's own composer is absent must not read as observed.
    ///
    /// Each hostile capture is DERIVED from the measured fixture beside it by
    /// removing only the composer, so the footer, the rule and the row order
    /// the grammar anchors on are the recorded bytes. The ungated read is
    /// asserted to still see a model on every one of them — without that, a
    /// mutant that deletes the gate would have nothing to fail.
    #[test]
    fn a_quoted_frame_without_the_tools_composer_is_not_observed() {
        const DRAFT: &str = " half a thought";
        let muse = include_str!("../tests/fixtures/runtime-identity/muse-idle-plain-80x24.txt");
        // Drop the composer row and the rule that opens it; the closing rule
        // and the footer under it — the whole of what `RuleFooter` reads —
        // stay exactly where the capture put them.
        let quoted_muse: String = muse
            .lines()
            .filter(|line| !line.trim_start().starts_with('\u{276f}'))
            .collect::<Vec<_>>()
            .join("\n");
        let grok = include_str!("../tests/fixtures/grok-composer/grok-composed-frame-80x24.txt");
        // Grok's border IS its identity row, so the composer cannot be taken
        // away without taking the model with it. A DRAFT is the honest second
        // case: the box and the label both survive one, and `composed_ui`
        // refuses it because a paste would merge with the human's text. The
        // refusal is that rule and not broken geometry — the ungated assert
        // below proves the bottom edge and its measured slack are still
        // exactly where `current_grok_identity` demands them, and the same
        // capture WITHOUT the draft is pinned observed in the test beneath
        // this one.
        let drafted_grok: String = grok
            .lines()
            .map(|line| match line.split_once('\u{276f}') {
                // The rail row's padding absorbs the draft, so the box keeps
                // the width the capture measured.
                Some((head, pad)) if pad.len() >= DRAFT.len() => {
                    format!("{head}\u{276f}{DRAFT}{}", &pad[DRAFT.len()..])
                }
                _ => line.to_owned(),
            })
            .collect::<Vec<_>>()
            .join("\n");

        for (name, capture, tool) in [
            ("muse", quoted_muse.as_str(), ToolKind::Muse),
            ("grok", drafted_grok.as_str(), ToolKind::Grok),
            (
                "agy",
                include_str!("../tests/fixtures/agy-composer/agy-draft-frame-80x24.txt"),
                ToolKind::Agy,
            ),
        ] {
            assert!(
                current_identity(capture, tool).model.is_some(),
                "{name}: the ungated grammar must still read a model here, or this \
                 fixture proves nothing about the gate"
            );
            assert_eq!(
                observed_identity(capture, tool),
                HarnessIdentity::default(),
                "{name}: a frame without the tool's own live composer is unobserved"
            );
        }
    }

    /// The gate must not cost a single live frame its identity. Every measured
    /// composer fixture reads the same through it as around it.
    #[test]
    fn every_modelled_tool_still_observes_through_the_composer_gate() {
        for (name, capture, tool) in [
            (
                "claude",
                include_str!("../tests/fixtures/harness-state/claude-idle-167x40.txt"),
                ToolKind::Claude,
            ),
            (
                "codex",
                include_str!("../tests/fixtures/harness-state/codex-idle-112x40.txt"),
                ToolKind::Codex,
            ),
            (
                "muse",
                include_str!("../tests/fixtures/runtime-identity/muse-idle-plain-80x24.txt"),
                ToolKind::Muse,
            ),
            (
                "muse-occupied",
                include_str!("../tests/fixtures/runtime-identity/muse-occupied-plain-80x24.txt"),
                ToolKind::Muse,
            ),
            (
                "opencode",
                include_str!(
                    "../tests/fixtures/runtime-identity/opencode-composed-plain-80x24.txt"
                ),
                ToolKind::OpenCode,
            ),
            (
                "grok",
                include_str!("../tests/fixtures/grok-composer/grok-composed-frame-80x24.txt"),
                ToolKind::Grok,
            ),
            (
                "agy",
                include_str!("../tests/fixtures/agy-composer/agy-composed-frame.txt"),
                ToolKind::Agy,
            ),
        ] {
            assert_eq!(
                observed_identity(capture, tool),
                current_identity(capture, tool),
                "{name}: the gate changed a live frame's answer"
            );
        }
        // An unmodelled tool proves nothing either way, gated or not.
        assert_eq!(
            observed_identity(
                include_str!("../tests/fixtures/harness-state/claude-idle-167x40.txt"),
                ToolKind::Gemini
            ),
            HarnessIdentity::default()
        );
    }

    #[test]
    fn the_effort_vocabulary_is_closed_and_published() {
        for word in ["low", "medium", "high", "xhigh", "max", "ultra"] {
            assert!(is_effort_word(word), "{word}");
        }
        for word in ["", "LOW", "lowest", "med", "xxhigh", "max "] {
            assert!(!is_effort_word(word), "{word:?}");
        }
        assert_eq!(
            "medium".len(),
            6,
            "the widest effort word, which is what the fact's cap and the \
             picker's column are sized to"
        );
    }
}
