//! The pure half of in-place seat compaction: the outcome vocabulary (R5), the
//! R7 degrade gate, the audit records with their start-of-run warning (4a),
//! and the P1 report renderer. Nothing here reads the world; bytes and facts
//! come in as arguments.

use crate::deliver::SubmitState;

/// The first word of an attempted dispatch.
pub const DISPATCHED: &str = "dispatched";
/// The first word of a seat the run never touched.
pub const SKIPPED: &str = "skipped";
/// The first word of a seat whose command may sit unsubmitted in its composer.
pub const NOT_DISPATCHED: &str = "not dispatched";

/// What `dispatched` claims, and deliberately what it does not. The help text
/// asserts this sentence.
pub const DISPATCH_DEFINITION: &str = "dispatched means attempted: the command was pasted and Enter was sent; it is never proof of submission";

/// The reason word a still-staged paste maps to.
pub const STAGED_TEXT: &str = "staged text";
/// The reason word the guarded operation's known `send_key` failure maps to.
pub const ENTER_FAILED: &str = "enter failed";
/// A seat the R3 fixed-only rule refuses.
pub const SPAWNED: &str = "spawned";
/// A seat whose tool has no drivable compaction command.
pub const UNSUPPORTED: &str = "unsupported";
/// A seat whose input box ae has no grammar for.
pub const INPUT_NOT_MODELLED: &str = "input not modelled";
/// A seat that dropped to a shell.
pub const DEAD: &str = "dead";
/// A seat holding human input.
pub const BUSY: &str = "busy";

/// One refusal leg of the R1 boundary: the offending VALUE is never printed.
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

/// What one seat.s turn ended as: exactly three words, no completion claim.
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

    /// The human.s next step when a paste may sit in a composer.
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

    #[test]
    fn the_vocabulary_is_exactly_the_three_words() {
        let words = [
            Outcome::skipped(BUSY).word(),
            Outcome::from_verdict(Verdict::State(SubmitState::Submitted), "%1").word(),
            Outcome::from_verdict(Verdict::State(SubmitState::StillStaged), "%1").word(),
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
        let submitted = Outcome::from_verdict(Verdict::State(SubmitState::Submitted), "%1");
        let unknown = Outcome::from_verdict(
            Verdict::State(SubmitState::Unknown(Unverifiable::CaptureUnreadable)),
            "%1",
        );
        assert_eq!(submitted.word(), DISPATCHED);
        assert_eq!(submitted.unverifiable(), None);
        assert_eq!(unknown.word(), DISPATCHED);
        assert_eq!(unknown.reason(), None);
        assert!(!unknown.word().contains("unverifiable"));
    }

    #[test]
    fn the_unknown_proof_gap_rides_the_record_alone() {
        let unknown = Outcome::from_verdict(
            Verdict::State(SubmitState::Unknown(Unverifiable::PaneUnparseable)),
            "%1",
        );
        assert_eq!(unknown.unverifiable(), Some("unparseable-input"));
        assert!(!unknown.word().contains("unparseable-input"));
    }

    #[test]
    fn a_staged_paste_is_not_dispatched_and_names_its_remedy() {
        let staged = Outcome::from_verdict(Verdict::State(SubmitState::StillStaged), "%3");
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
}
