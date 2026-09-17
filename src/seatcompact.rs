//! The pure half of in-place seat compaction: the R5 vocabulary, the R7 gate,
//! the 4a audit records with their R1 warning, and the P1 report. Nothing here
//! reads the world; bytes and facts come in as arguments.

use crate::deliver::SubmitState;
use crate::tool::InputModel;

pub const DISPATCHED: &str = "dispatched";
pub const SKIPPED: &str = "skipped";
pub const NOT_DISPATCHED: &str = "not dispatched";
pub const STAGED_TEXT: &str = "staged text";
pub const ENTER_FAILED: &str = "enter failed";
pub const SPAWNED: &str = "spawned";
pub const UNSUPPORTED: &str = "unsupported";
pub const INPUT_NOT_MODELLED: &str = "input not modelled";
pub const DEAD: &str = "dead";
pub const BUSY: &str = "busy";

/// What `dispatched` claims, and what it never claims. The help text asserts
/// this sentence.
pub const DISPATCH_DEFINITION: &str = "dispatched means attempted: the command was pasted and Enter was sent; it is never proof of submission";

/// One refusal leg of the R1 boundary. The offending VALUE is never printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapLeg {
    Unreadable,
    Vacant,
    Mismatch,
    Live,
}

impl GapLeg {
    /// The leg as a constant.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unreadable => "unreadable",
            Self::Vacant => "vacant",
            Self::Mismatch => "mismatch",
            Self::Live => "live",
        }
    }
}

/// What one seat's turn ended as: exactly three words, no completion claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Skipped {
        reason: String,
    },
    Dispatched {
        unverifiable: Option<&'static str>,
    },
    NotDispatched {
        reason: String,
        pane: Option<String>,
    },
}

/// What the guarded operation returned: the raw verdict or its known failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    State(SubmitState),
    EnterFailed,
}

impl Outcome {
    /// Map the verdict; `pane` is the paste target the remedy names.
    #[must_use]
    pub fn from_verdict(verdict: Verdict, pane: &str) -> Self {
        match verdict {
            Verdict::EnterFailed => Self::NotDispatched {
                reason: ENTER_FAILED.to_owned(),
                pane: None,
            },
            Verdict::State(SubmitState::Submitted) => Self::Dispatched { unverifiable: None },
            Verdict::State(SubmitState::Unknown(reason)) => Self::Dispatched {
                unverifiable: Some(reason.event_marker()),
            },
            Verdict::State(SubmitState::StillStaged) => Self::NotDispatched {
                reason: STAGED_TEXT.to_owned(),
                pane: Some(pane.to_owned()),
            },
        }
    }

    /// The refusal of an identity gap, named by its leg.
    #[must_use]
    pub fn identity_gap(leg: GapLeg) -> Self {
        Self::Skipped {
            reason: format!("identity gap: {}", leg.as_str()),
        }
    }

    /// A skip with a constant reason.
    #[must_use]
    pub fn skipped(reason: &str) -> Self {
        Self::Skipped {
            reason: reason.to_owned(),
        }
    }

    /// The word, parentheses included.
    #[must_use]
    pub fn word(&self) -> String {
        match self {
            Self::Skipped { reason } => format!("{SKIPPED} ({reason})"),
            Self::Dispatched { .. } => DISPATCHED.to_owned(),
            Self::NotDispatched { reason, .. } => format!("{NOT_DISPATCHED} ({reason})"),
        }
    }

    /// The additive `reason=` text, for skips and unsubmitted pastes.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Skipped { reason } | Self::NotDispatched { reason, .. } => Some(reason),
            Self::Dispatched { .. } => None,
        }
    }

    /// The proof gap `Unknown` carries on the RECORD alone.
    #[must_use]
    pub fn unverifiable(&self) -> Option<&'static str> {
        match self {
            Self::Dispatched { unverifiable } => *unverifiable,
            _ => None,
        }
    }

    /// The human's next step when a paste may sit in a composer.
    #[must_use]
    pub fn remedy(&self) -> Option<String> {
        match self {
            Self::NotDispatched {
                reason,
                pane: Some(pane),
            } if reason == STAGED_TEXT => {
                Some(format!("clear the composer of {pane} before any send"))
            }
            _ => None,
        }
    }
}

/// A LOCAL mirror of the R11 adapter row's compaction capability, until the
/// adapter carries one; C2b replaces it with the adapter's own enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactSpec {
    Guided { command: &'static str },
    Bare { command: &'static str },
    Unsupported { reason: &'static str },
}

/// The arm a modelled seat dispatches with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Guided,
    Bare,
}

/// One seat's R7 verdict. `hand_remedy` marks the gate-(2) skip alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Admit(Mode),
    Refuse {
        reason: &'static str,
        hand_remedy: bool,
    },
}

/// R7's gates in order: spawned, `Unsupported`, `Unmodelled`; else admit the
/// spec's arm. It matches [`InputModel`] and the spec, never a tool kind.
#[must_use]
pub fn classify(spec: CompactSpec, input: InputModel, spawned: bool) -> Gate {
    if spawned {
        return Gate::Refuse {
            reason: SPAWNED,
            hand_remedy: false,
        };
    }
    let mode = match spec {
        CompactSpec::Unsupported { .. } => {
            return Gate::Refuse {
                reason: UNSUPPORTED,
                hand_remedy: false,
            };
        }
        CompactSpec::Guided { .. } => Mode::Guided,
        CompactSpec::Bare { .. } => Mode::Bare,
    };
    match input {
        InputModel::Unmodelled => Gate::Refuse {
            reason: INPUT_NOT_MODELLED,
            hand_remedy: true,
        },
        InputModel::BorderDelimited | InputModel::StyleDelimited => Gate::Admit(mode),
    }
}

/// The projection budget: every valid value fits (a slot stops at 64, a
/// request id at 32), so a cut is hostile input being said out loud.
pub const CELLS: usize = 64;

/// The ONE display projection of record-derived text: printable ASCII, escapes
/// folded, bounded to [`CELLS`].
#[must_use]
pub fn cell(text: &str) -> String {
    crate::event_text::display_cell(text, CELLS)
}

/// The `compact by hand:` line: the gate-(2) seats grouped by tool, in roster
/// order, each slot projected and each tool named. `None` when none was.
#[must_use]
pub fn hand_remedy_line(seats: &[(&str, &str, Gate)]) -> Option<String> {
    let mut groups: Vec<(&str, Vec<&str>)> = Vec::new();
    for (slot, tool, gate) in seats {
        if !matches!(
            gate,
            Gate::Refuse {
                hand_remedy: true,
                ..
            }
        ) {
            continue;
        }
        match groups.iter_mut().find(|(named, _)| named == tool) {
            Some((_, slots)) => slots.push(slot),
            None => groups.push((tool, vec![slot])),
        }
    }
    if groups.is_empty() {
        return None;
    }
    let named: Vec<String> = groups
        .iter()
        .map(|(tool, slots)| {
            let slots: Vec<String> = slots.iter().map(|slot| cell(slot)).collect();
            format!(
                "{} ({}: {INPUT_NOT_MODELLED})",
                slots.join(", "),
                cell(tool)
            )
        })
        .collect();
    Some(format!("compact by hand: {}", named.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deliver::Unverifiable;

    /// This module's own source, comments stripped, TESTS EXCLUDED.
    fn production() -> String {
        let source = include_str!("seatcompact.rs");
        let code: String = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let (module, tests) = code.split_once("#[cfg(test)]").expect("a test module");
        assert!(tests.contains("fn production"));
        assert!(
            module.contains("pub enum Outcome"),
            "the scan reached the code"
        );
        module.to_owned()
    }

    const GUIDED: CompactSpec = CompactSpec::Guided {
        command: "/compact",
    };
    const BARE: CompactSpec = CompactSpec::Bare {
        command: "/compact",
    };
    const NONE: CompactSpec = CompactSpec::Unsupported {
        reason: "print-mode only",
    };

    fn outcome(state: SubmitState) -> Outcome {
        Outcome::from_verdict(Verdict::State(state), "%3")
    }

    #[test]
    fn the_vocabulary_is_exactly_the_three_words() {
        let words = [
            Outcome::skipped(BUSY).word(),
            outcome(SubmitState::Submitted).word(),
            outcome(SubmitState::StillStaged).word(),
        ];
        let first: Vec<&str> = words
            .iter()
            .map(|word| word.split(" (").next().unwrap_or_default())
            .collect();
        assert_eq!(first, [SKIPPED, DISPATCHED, NOT_DISPATCHED]);
        assert!(
            !production().contains("compacted"),
            "R5 claims no completion"
        );
    }

    #[test]
    fn submitted_and_unknown_both_render_dispatched() {
        let submitted = outcome(SubmitState::Submitted);
        let unknown = outcome(SubmitState::Unknown(Unverifiable::CaptureUnreadable));
        assert_eq!(submitted.word(), DISPATCHED);
        assert_eq!(submitted.unverifiable(), None);
        assert_eq!(unknown.word(), DISPATCHED);
        assert_eq!(unknown.reason(), None);
        assert_eq!(unknown.unverifiable(), Some("unreadable-capture"));
        assert!(!unknown.word().contains("unreadable-capture"));
    }

    #[test]
    fn the_unknown_proof_gap_rides_the_record_alone() {
        let unknown = outcome(SubmitState::Unknown(Unverifiable::PaneUnparseable));
        assert_eq!(unknown.unverifiable(), Some("unparseable-input"));
        assert!(!unknown.word().contains("unparseable-input"));
    }

    #[test]
    fn a_staged_paste_is_not_dispatched_and_names_its_remedy() {
        let staged = outcome(SubmitState::StillStaged);
        assert_eq!(staged.word(), "not dispatched (staged text)");
        assert_eq!(staged.reason(), Some(STAGED_TEXT));
        assert_eq!(
            staged.remedy().as_deref(),
            Some("clear the composer of %3 before any send")
        );
    }

    #[test]
    fn a_failed_enter_is_its_own_word() {
        let failed = Outcome::from_verdict(Verdict::EnterFailed, "%3");
        assert_eq!(failed.word(), "not dispatched (enter failed)");
        assert_eq!(failed.reason(), Some(ENTER_FAILED));
        assert_eq!(failed.unverifiable(), None);
    }

    #[test]
    fn an_identity_gap_names_only_its_leg() {
        for (leg, text) in [
            (GapLeg::Unreadable, "identity gap: unreadable"),
            (GapLeg::Vacant, "identity gap: vacant"),
            (GapLeg::Mismatch, "identity gap: mismatch"),
            (GapLeg::Live, "identity gap: live"),
        ] {
            assert_eq!(
                Outcome::identity_gap(leg).word(),
                format!("skipped ({text})")
            );
        }
    }

    #[test]
    fn the_definition_says_attempted_and_never_proof() {
        assert_eq!(
            DISPATCH_DEFINITION,
            "dispatched means attempted: the command was pasted and Enter was sent; \
             it is never proof of submission"
        );
    }

    #[test]
    fn unsupported_outranks_unmodelled_and_is_never_on_the_hand_line() {
        let agy = classify(NONE, InputModel::Unmodelled, false);
        assert_eq!(
            agy,
            Gate::Refuse {
                reason: UNSUPPORTED,
                hand_remedy: false
            }
        );
        assert_eq!(hand_remedy_line(&[("w1", "agy", agy)]), None);
    }

    #[test]
    fn an_unmodelled_seat_is_named_for_hand_compaction_with_its_tool() {
        let opencode = classify(BARE, InputModel::Unmodelled, false);
        assert_eq!(
            opencode,
            Gate::Refuse {
                reason: INPUT_NOT_MODELLED,
                hand_remedy: true
            }
        );
        let grok = classify(GUIDED, InputModel::Unmodelled, false);
        let claude = classify(GUIDED, InputModel::BorderDelimited, false);
        assert_eq!(claude, Gate::Admit(Mode::Guided));
        assert_eq!(
            hand_remedy_line(&[
                ("w1", "opencode", opencode),
                ("w2", "grok", grok),
                ("w3", "claude", claude)
            ])
            .as_deref(),
            Some(
                "compact by hand: w1 (opencode: input not modelled), \
                  w2 (grok: input not modelled)"
            )
        );
    }

    #[test]
    fn gate_two_seats_share_one_parenthetical_per_tool() {
        let gate = classify(BARE, InputModel::Unmodelled, false);
        assert_eq!(
            hand_remedy_line(&[("a", "opencode", gate), ("b", "opencode", gate)]).as_deref(),
            Some("compact by hand: a, b (opencode: input not modelled)")
        );
    }

    #[test]
    fn a_spawned_seat_is_refused_before_every_other_gate() {
        let spawned = classify(NONE, InputModel::Unmodelled, true);
        assert_eq!(
            spawned,
            Gate::Refuse {
                reason: SPAWNED,
                hand_remedy: false
            }
        );
        assert_eq!(
            classify(GUIDED, InputModel::BorderDelimited, true),
            Gate::Refuse {
                reason: SPAWNED,
                hand_remedy: false
            }
        );
    }

    #[test]
    fn a_modelled_seat_is_admitted_with_its_specs_arm() {
        assert_eq!(
            classify(GUIDED, InputModel::BorderDelimited, false),
            Gate::Admit(Mode::Guided)
        );
        assert_eq!(
            classify(BARE, InputModel::StyleDelimited, false),
            Gate::Admit(Mode::Bare)
        );
    }

    #[test]
    fn the_module_source_guards_hold() {
        let production = production();
        assert!(!production.contains("ToolKind"), "R7 reads the input model");
        assert_eq!(
            production.matches("display_cell").count(),
            1,
            "one projection owner"
        );
    }
}
