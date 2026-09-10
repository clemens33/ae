//! Pure classification of the current harness frame already captured by the watchdog.

use crate::tool::{InputModel, ToolKind};

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
        "{frame}:{}:{}:{}:{}",
        carry.since_epoch, carry.nudges, carry.undelivered, carry.identity,
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
    let lines = clean_lines(capture);
    match model {
        InputModel::StyleDelimited => lines
            .iter()
            .rfind(|line| line.starts_with('›'))
            .is_some_and(|line| *line != "› Ask Codex to do anything"),
        InputModel::BorderDelimited => lines
            .iter()
            .rfind(|line| line.starts_with('❯'))
            .is_some_and(|line| *line != "❯"),
        InputModel::Unmodelled => false,
    }
}

fn clean_lines(capture: &str) -> Vec<&str> {
    capture
        .lines()
        .map(|line| line.trim_matches([' ', '\t', '\r', '\u{a0}']))
        .filter(|line| !line.is_empty())
        .collect()
}

fn classify_codex(capture: &str) -> HarnessState {
    let lines = clean_lines(capture);
    let Some((before, [prompt, footer])) = lines.as_slice().split_last_chunk::<2>() else {
        return HarnessState::Unknown;
    };
    if *prompt != "› Ask Codex to do anything" || !codex_footer(footer) {
        return HarnessState::Unknown;
    }
    if before.last().is_some_and(|line| codex_modal(line)) {
        return HarnessState::Unknown;
    }
    if before
        .last()
        .is_some_and(|line| line.starts_with("• Working (") && line.ends_with("esc to interrupt)"))
    {
        HarnessState::Busy
    } else {
        HarnessState::Idle
    }
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
    if lines[prompt_index + 1..]
        .iter()
        .any(|line| !claude_chrome(line))
    {
        return HarnessState::Unknown;
    }
    let current = lines[..prompt_index]
        .iter()
        .rev()
        .find(|line| !claude_chrome(line));
    match current {
        Some(line) if claude_spinner(line) => HarnessState::Busy,
        Some(line) if claude_done(line) => HarnessState::Idle,
        None => HarnessState::Idle,
        _ => HarnessState::Unknown,
    }
}

fn claude_spinner(line: &str) -> bool {
    let mut chars = line.chars();
    matches!(chars.next(), Some('✻' | '✽' | '✳' | '✢' | '·')) && chars.as_str().contains("… (")
}

fn claude_done(line: &str) -> bool {
    line.starts_with("✻ ") && line.contains(" · done ")
}

fn claude_chrome(line: &str) -> bool {
    line.chars().all(|c| matches!(c, '─' | '━' | '═'))
        || line.starts_with('🧠')
        || line.starts_with('⏵')
        || line.starts_with('✔')
        || line.starts_with("⎿  Tip:")
}

fn codex_footer(line: &str) -> bool {
    let Some((model_effort, path)) = line.split_once(" · ") else {
        return false;
    };
    let mut words = model_effort.split_whitespace();
    let model = words.next().unwrap_or_default();
    let effort = words.next().unwrap_or_default();
    words.next().is_none()
        && model.starts_with("gpt-")
        && matches!(
            effort,
            "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
        )
        && !path.is_empty()
}

#[cfg(test)]
mod tests {
    use super::{
        HarnessState, IdleCarry, classify, decode_idle, encode_idle, has_human_draft,
        observed_from_option,
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

    #[test]
    fn a_resumed_live_claude_done_frame_is_idle() {
        let capture =
            include_str!("../tests/fixtures/harness-state/claude-resumed-idle-149x37.txt");
        assert_eq!(classify(capture, ToolKind::Claude), HarnessState::Idle);
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
        assert!(!has_human_draft(
            include_str!("../tests/fixtures/harness-state/codex-busy-280x40.txt"),
            ToolKind::Codex
        ));
        assert!(!has_human_draft("❯ draft", ToolKind::Grok));
    }

    #[test]
    fn observed_option_is_total_and_idle_carry_round_trips() {
        let carry = IdleCarry {
            since_epoch: 42,
            nudges: 2,
            undelivered: 1,
            identity: 99,
        };
        let encoded = encode_idle(carry);
        assert_eq!(decode_idle(&encoded), Some(carry));
        assert_eq!(observed_from_option(&encoded), HarnessState::Idle);
        assert_eq!(observed_from_option("busy"), HarnessState::Busy);
        for malformed in ["", "idle:x:2:1:99", "idle:42:2:1", "busy:junk", "modal"] {
            assert_eq!(
                observed_from_option(malformed),
                HarnessState::Unknown,
                "{malformed}"
            );
            assert_eq!(decode_idle(malformed), None, "{malformed}");
        }
    }
}
