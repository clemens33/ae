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
    let lines = clean_lines(capture);
    match model {
        InputModel::StyleDelimited => {
            let Some((before, [prompt, footer])) = lines.as_slice().split_last_chunk::<2>() else {
                return false;
            };
            codex_footer(footer)
                && !before.last().is_some_and(|line| codex_modal(line))
                && prompt.starts_with('›')
                && *prompt != "› Ask Codex to do anything"
        }
        InputModel::BorderDelimited => lines
            .iter()
            .rposition(|line| line.starts_with('❯'))
            .is_some_and(|index| claude_input_frame(&lines, index) && lines[index] != "❯"),
        InputModel::Unmodelled => false,
    }
}

/// Extract model and effort independently from the positively bounded current
/// frame. This never searches transcript text or historical rows.
#[must_use]
pub fn current_identity(capture: &str, tool: ToolKind) -> HarnessIdentity {
    match tool.input_model() {
        InputModel::BorderDelimited => current_claude_identity(capture),
        InputModel::StyleDelimited => current_codex_identity(capture),
        InputModel::Unmodelled => HarnessIdentity::default(),
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
        None => HarnessState::Idle,
        _ => HarnessState::Unknown,
    }
}

fn claude_input_frame(lines: &[&str], prompt_index: usize) -> bool {
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
    matches!(chars.next(), Some('✻' | '✽' | '✳' | '✢' | '·')) && chars.as_str().contains("… (")
}

fn claude_done(line: &str) -> bool {
    line.starts_with("✻ ") && line.contains(" · done ")
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
    model.starts_with("gpt-") && valid_effort(effort) && !path.is_empty()
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

const CLAUDE_MODELS: [&str; 4] = ["Fable 5.1", "Opus 4.8", "Opus 5 (1M context)", "Opus 5"];

fn current_codex_identity(capture: &str) -> HarnessIdentity {
    let lines = clean_lines(capture);
    let Some((before, [prompt, footer])) = lines.as_slice().split_last_chunk::<2>() else {
        return HarnessIdentity::default();
    };
    if *prompt != "› Ask Codex to do anything"
        || codex_footer_parts(footer).is_none()
        || before.last().is_some_and(|line| codex_modal(line))
    {
        return HarnessIdentity::default();
    }
    parse_codex_identity(footer)
}

fn codex_footer_parts(line: &str) -> Option<(&str, &str, &str)> {
    let (model_effort, path) = line.split_once(" · ")?;
    let mut words = model_effort.split_whitespace();
    let model = words.next()?;
    let effort = words.next()?;
    (words.next().is_none() && !path.is_empty()).then_some((model, effort, path))
}

fn parse_codex_identity(line: &str) -> HarnessIdentity {
    let Some((model_token, effort_token, _path)) = codex_footer_parts(line) else {
        return HarnessIdentity::default();
    };
    let model = matches!(
        model_token,
        "gpt-5.6-luna" | "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-6-astra"
    )
    .then(|| model_token.to_owned());
    let effort = valid_effort(effort_token).then(|| effort_token.to_owned());
    HarnessIdentity { model, effort }
}

fn valid_effort(value: &str) -> bool {
    matches!(value, "low" | "medium" | "high" | "xhigh" | "max" | "ultra")
}

#[cfg(test)]
mod tests {
    use super::{
        HarnessIdentity, HarnessState, IdleCarry, classify, current_identity, decode_idle,
        encode_idle, has_human_draft, observed_from_option,
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
}
