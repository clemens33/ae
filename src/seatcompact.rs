//! The pure half of in-place seat compaction: the R5 vocabulary, the R7 gate,
//! the 4a audit records with their R1 warning, and the P1 report. Nothing here
//! reads the world; bytes and facts come in as arguments.

use crate::deliver::SubmitState;
use crate::event_text::{self as text, extract, read_lines};
use crate::harness_state::{FrameReading, HarnessState};
use crate::json::Value;
use crate::state::{event_line, summary_of};
use crate::time::Timestamp;
pub use crate::tool::CompactSpec;
use crate::tool::InputModel;
use std::fmt::Write as _;
use std::str;

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
pub const TARGET_LOCKED: &str = "target locked";
pub const LIFECYCLE_LOCKED: &str = "lifecycle locked";
pub const PASTE_FAILED: &str = "paste failed";
pub const OBSERVED_IDLE: &str = "observed idle";
pub const UNOBSERVED: &str = "unobserved";
pub const FRAME_NOT_MODELLED: &str = "frame not modelled";
pub const BASELINE_UNKNOWN: &str = "baseline unknown";
pub const RELAUNCHED: &str = "relaunched";
pub const TIMEOUT: &str = "timeout";

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
            } if reason == STAGED_TEXT => Some(format!(
                "clear the composer of {} before any send",
                cell(pane)
            )),
            _ => None,
        }
    }
}

/// What the frames after a dispatch showed: readiness, never proof that the
/// command ran, or why readiness was not observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    Idle,
    Unobserved { reason: String },
}

impl Observation {
    /// An unobserved end with a constant reason.
    #[must_use]
    pub fn unobserved(reason: &str) -> Self {
        Self::Unobserved {
            reason: reason.to_owned(),
        }
    }

    /// An identity gap that ended the watch, named by its leg.
    #[must_use]
    pub fn identity_gap(leg: GapLeg) -> Self {
        Self::Unobserved {
            reason: format!("identity gap: {}", leg.as_str()),
        }
    }

    /// The words the report puts in parentheses after `dispatched`.
    #[must_use]
    pub fn word(&self) -> String {
        match self {
            Self::Idle => OBSERVED_IDLE.to_owned(),
            Self::Unobserved { reason } => format!("{UNOBSERVED}: {reason}"),
        }
    }
}

/// One post-dispatch sample, already judged against the guards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sample {
    /// A guard ended the watch: identity, stamp, liveness or the ceiling.
    Ended(Observation),
    /// A read that proves nothing: liveness unproven or no capture.
    Blind,
    /// The frame, `None` when its grammar recognized none.
    Frame(Option<FrameReading>),
}

/// The fold: two CONSECUTIVE idle frames whose judged row differs from the
/// one read before the dispatch prove idle; any other sample restarts the pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watch {
    baseline: Option<String>,
    streak: u8,
}

impl Watch {
    /// Start from the frame read before the dispatch.
    ///
    /// # Errors
    ///
    /// [`BASELINE_UNKNOWN`] when no grammar recognized that frame: nothing
    /// after it could be told apart from it.
    pub fn new(baseline: Option<FrameReading>) -> Result<Self, Observation> {
        match baseline {
            Some(frame) => Ok(Self {
                baseline: frame.current,
                streak: 0,
            }),
            None => Err(Observation::unobserved(BASELINE_UNKNOWN)),
        }
    }

    /// Fold one sample; `Some` ends the watch.
    pub fn feed(&mut self, sample: Sample) -> Option<Observation> {
        let qualifying = match sample {
            Sample::Ended(end) => return Some(end),
            Sample::Blind | Sample::Frame(None) => false,
            Sample::Frame(Some(frame)) => {
                frame.state == HarnessState::Idle && frame.current != self.baseline
            }
        };
        self.streak = if qualifying {
            self.streak.saturating_add(1)
        } else {
            0
        };
        (self.streak >= 2).then_some(Observation::Idle)
    }
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

/// The seat action and the run action of the audit ledger.
pub const ACTION: &str = "seat-compact";
pub const RUN_ACTION: &str = "seat-compact-run";
/// The run record's two summary words.
pub const RUN_START: &str = "start";
pub const RUN_END: &str = "end";

/// One seat's audit record: the outcome plus the facts that bind it to the run
/// and the target.
#[derive(Debug, Clone, Copy)]
pub struct SeatRecord<'a> {
    pub ts: Timestamp,
    pub actor: &'a str,
    pub run: &'a str,
    pub request: &'a str,
    pub slot: &'a str,
    pub session: &'a str,
    pub outcome: &'a Outcome,
    pub sanitized: usize,
}

/// The `seat-compact` event line: [`crate::state::event_line`]'s shape, plus
/// the run, the target and the verdict's additive fields.
#[must_use]
pub fn seat_record(record: &SeatRecord<'_>) -> String {
    let mut members = vec![
        ("ts".to_owned(), Value::Str(record.ts.to_string())),
        ("actor".to_owned(), Value::Str(record.actor.to_owned())),
        ("action".to_owned(), Value::Str(ACTION.to_owned())),
    ];
    if !record.request.is_empty() {
        members.push(("ref".to_owned(), Value::Str(record.request.to_owned())));
    }
    let summary = summary_of(&format!("{} {}", record.outcome.word(), record.slot));
    members.push(("summary".to_owned(), Value::Str(summary)));
    for (key, text) in [
        ("run", record.run),
        ("target_slot", record.slot),
        ("target_session", record.session),
    ] {
        if !text.is_empty() {
            members.push((key.to_owned(), Value::Str(text.to_owned())));
        }
    }
    if let Some(marker) = record.outcome.unverifiable() {
        members.push(("unverifiable".to_owned(), Value::Str(marker.to_owned())));
    }
    if let Some(reason) = record.outcome.reason() {
        members.push(("reason".to_owned(), Value::Str(reason.to_owned())));
    }
    if record.sanitized > 0 {
        members.push((
            "sanitized".to_owned(),
            Value::Num(i64::try_from(record.sanitized).unwrap_or(i64::MAX)),
        ));
    }
    let mut line = Value::Obj(members).render();
    line.push('\n');
    line
}

/// The `seat-compact-run` event line.
#[must_use]
pub fn run_record(ts: Timestamp, actor: &str, run: &str, phase: &str) -> String {
    event_line(ts, actor, RUN_ACTION, run, phase)
}

/// The observation's audit action, appended after the seat's `seat-compact`
/// record and never in its place.
pub const OBSERVED_ACTION: &str = "seat-compact-observed";

/// One seat's observation record: the seat record's binding facts with the
/// observation in the outcome's place. It names no `target`, so it addresses
/// no seat and ends no quiet state.
#[derive(Debug, Clone, Copy)]
pub struct ObservedRecord<'a> {
    pub ts: Timestamp,
    pub actor: &'a str,
    pub run: &'a str,
    pub request: &'a str,
    pub slot: &'a str,
    pub session: &'a str,
    pub observation: &'a Observation,
}

/// The `seat-compact-observed` event line.
#[must_use]
pub fn observed_record(record: &ObservedRecord<'_>) -> String {
    let word = match record.observation {
        Observation::Idle => OBSERVED_IDLE,
        Observation::Unobserved { .. } => UNOBSERVED,
    };
    let mut members = vec![
        ("ts".to_owned(), Value::Str(record.ts.to_string())),
        ("actor".to_owned(), Value::Str(record.actor.to_owned())),
        ("action".to_owned(), Value::Str(OBSERVED_ACTION.to_owned())),
    ];
    if !record.request.is_empty() {
        members.push(("ref".to_owned(), Value::Str(record.request.to_owned())));
    }
    let summary = summary_of(&format!("{word} {}", record.slot));
    members.push(("summary".to_owned(), Value::Str(summary)));
    for (key, text) in [
        ("run", record.run),
        ("target_slot", record.slot),
        ("target_session", record.session),
    ] {
        if !text.is_empty() {
            members.push((key.to_owned(), Value::Str(text.to_owned())));
        }
    }
    if let Observation::Unobserved { reason } = record.observation {
        members.push(("reason".to_owned(), Value::Str(reason.clone())));
    }
    let mut line = Value::Obj(members).render();
    line.push('\n');
    line
}

/// A compact elapsed span (`59s`, `3m`, `2h`, `4d`); a negative delta reads
/// `0s`.
#[must_use]
pub fn span(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h", seconds / 3_600)
    } else {
        format!("{}d", seconds / 86_400)
    }
}

/// The R1 start-of-run warning, pure: bytes and `now` in, one line out for
/// every seat of THIS actor whose `dispatched` record names a run with no
/// `end`. An unparseable `ts` omits its line; nothing gates on the result.
#[must_use]
pub fn audit_warning(bytes: &[u8], actor: &str, now: Timestamp) -> Vec<String> {
    let records: Vec<&[u8]> = read_lines(bytes)
        .into_iter()
        .filter_map(text::event_line)
        .collect();
    let mut out = Vec::new();
    for line in &records {
        if member_value(line, "actor").as_deref() != Some(actor)
            || member_value(line, "action").as_deref() != Some(ACTION)
        {
            continue;
        }
        let summary = extract(line, "summary");
        let word = summary
            .split(|byte| *byte == b' ')
            .next()
            .unwrap_or_default();
        if word != DISPATCHED.as_bytes() {
            continue;
        }
        let run = extract(line, "run");
        if run.is_empty() || run_ended(&records, actor, &run) {
            continue;
        }
        let Some(age) = dispatch_age(line, now) else {
            continue;
        };
        let slot = String::from_utf8_lossy(&extract(line, "target_slot")).into_owned();
        out.push(format!(
            "note: {} was dispatched {age} ago by a run that did not end",
            cell(&slot)
        ));
    }
    out
}

/// A flat member as text, or `None` when it is absent or not UTF-8.
fn member_value(line: &[u8], key: &str) -> Option<String> {
    str::from_utf8(&extract(line, key))
        .ok()
        .map(ToOwned::to_owned)
}

/// Whether the actor's own `end` record names this run.
fn run_ended(records: &[&[u8]], actor: &str, run: &[u8]) -> bool {
    records.iter().any(|line| {
        member_value(line, "actor").as_deref() == Some(actor)
            && member_value(line, "action").as_deref() == Some(RUN_ACTION)
            && member_value(line, "summary").as_deref() == Some(RUN_END)
            && extract(line, "ref").as_slice() == run
    })
}

/// The dispatched record's age, or `None` when its `ts` is unreadable.
fn dispatch_age(line: &[u8], now: Timestamp) -> Option<String> {
    let bytes = extract(line, "ts");
    let ts = Timestamp::parse(str::from_utf8(&bytes).ok()?)?;
    Some(span(now.epoch().saturating_sub(ts.epoch())))
}

/// One seat's report entry: what the hand line names, the gate verdict the
/// caller already took, the outcome, the elapsed seconds, and R4's cancelled
/// earlier checkpoint, if any, and what was observed after a dispatch.
#[derive(Debug, Clone, Copy)]
pub struct SeatLine<'a> {
    pub slot: &'a str,
    pub tool: &'a str,
    pub gate: Gate,
    pub outcome: &'a Outcome,
    pub elapsed: i64,
    pub earlier_checkpoint: Option<&'a str>,
    pub observation: Option<&'a Observation>,
}

/// The P1 report: one line per seat, then the `compact by hand:` line. Every
/// record-derived field is projected through [`cell`].
#[must_use]
pub fn report(lines: &[SeatLine<'_>]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str(&cell(&line.outcome.word()));
        if let Some(seen) = line.observation {
            let _ = write!(out, " ({})", cell(&seen.word()));
        }
        let _ = write!(out, " {} ({})", cell(line.slot), span(line.elapsed));
        if let Some(remedy) = line.outcome.remedy() {
            let _ = write!(out, " — {remedy}");
        }
        out.push('\n');
        if let Some(reference) = line.earlier_checkpoint {
            let _ = writeln!(out, "  note: earlier checkpoint {}", cell(reference));
        }
    }
    let gates: Vec<(&str, &str, Gate)> = lines.iter().map(|l| (l.slot, l.tool, l.gate)).collect();
    if let Some(hand) = hand_remedy_line(&gates) {
        out.push_str(&hand);
        out.push('\n');
    }
    out
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
        // A box that never cleared but did not POSITIVELY hold the paste.
        let unconfirmed = outcome(SubmitState::Unknown(Unverifiable::Unconfirmed));
        assert_eq!(unconfirmed.word(), DISPATCHED);
        assert_eq!(unconfirmed.unverifiable(), Some("unconfirmed-input"));
        assert_eq!(unconfirmed.remedy(), None);
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
        for wrapped in [
            "cell(&line.outcome.word())",
            "cell(&seen.word())",
            "cell(line.slot)",
            "cell(reference)",
            "cell(pane)",
            "cell(slot)",
            "cell(tool)",
        ] {
            assert!(production.contains(wrapped), "unwrapped: {wrapped}");
        }
    }

    const TS: Timestamp = Timestamp::from_epoch(1_787_000_000);
    const NOW: Timestamp = Timestamp::from_epoch(1_787_000_300);
    const ACTOR: &str = "ae:seats:u";
    const REQUEST: &str = "ae-20260917T120000Z-00000001";

    fn record<'a>(
        actor: &'a str,
        outcome: &'a Outcome,
        request: &'a str,
        sanitized: usize,
    ) -> String {
        seat_record(&SeatRecord {
            ts: TS,
            actor,
            run: "run-1",
            request,
            slot: "w1",
            session: "s",
            outcome,
            sanitized,
        })
    }

    #[test]
    fn a_seat_record_carries_the_word_the_run_and_the_additive_fields() {
        let unknown = Outcome::from_verdict(
            Verdict::State(SubmitState::Unknown(Unverifiable::CaptureUnreadable)),
            "%3",
        );
        let line = record(ACTOR, &unknown, REQUEST, 2);
        let bytes = line.as_bytes();
        assert_eq!(extract(bytes, "action"), ACTION.as_bytes());
        assert_eq!(extract(bytes, "summary"), b"dispatched w1");
        assert_eq!(extract(bytes, "ref"), REQUEST.as_bytes());
        assert_eq!(extract(bytes, "run"), b"run-1");
        assert_eq!(extract(bytes, "target_slot"), b"w1");
        assert_eq!(extract(bytes, "target_session"), b"s");
        assert_eq!(extract(bytes, "unverifiable"), b"unreadable-capture");
        assert!(line.contains("\"sanitized\":2"));
        assert!(!line.contains("\"reason\""));
    }

    #[test]
    fn a_skip_record_carries_its_leg_and_never_a_marker() {
        let line = record(ACTOR, &Outcome::identity_gap(GapLeg::Live), "", 0);
        let bytes = line.as_bytes();
        assert_eq!(
            extract(bytes, "summary"),
            b"skipped (identity gap: live) w1"
        );
        assert_eq!(extract(bytes, "reason"), b"identity gap: live");
        assert!(!line.contains("\"unverifiable\""));
        assert!(!line.contains("\"sanitized\""));
        assert!(!line.contains("\"ref\""), "a gate skip opened no request");
    }

    #[test]
    fn a_run_record_is_the_shared_event_shape() {
        assert_eq!(
            run_record(TS, ACTOR, "run-1", RUN_END),
            event_line(TS, ACTOR, RUN_ACTION, "run-1", RUN_END)
        );
    }

    fn audit(lines: &[String]) -> Vec<u8> {
        lines.concat().into_bytes()
    }

    #[test]
    fn a_dispatched_run_without_an_end_is_warned_and_an_end_silences_it() {
        let dispatched = Outcome::from_verdict(Verdict::State(SubmitState::Submitted), "%1");
        let seat = record(ACTOR, &dispatched, REQUEST, 0);
        let start = run_record(TS, ACTOR, "run-1", RUN_START);
        assert_eq!(
            audit_warning(&audit(&[start, seat.clone()]), ACTOR, NOW),
            ["note: w1 was dispatched 5m ago by a run that did not end"]
        );
        let end = run_record(TS, ACTOR, "run-1", RUN_END);
        assert!(audit_warning(&audit(&[seat, end]), ACTOR, NOW).is_empty());
    }

    #[test]
    fn a_trimmed_audit_and_another_actors_records_are_silent() {
        let dispatched = Outcome::from_verdict(Verdict::State(SubmitState::Submitted), "%1");
        let foreign = record("other", &dispatched, "", 0);
        assert!(audit_warning(&audit(&[foreign]), ACTOR, NOW).is_empty());
        assert!(audit_warning(b"", ACTOR, NOW).is_empty());
    }

    #[test]
    fn an_unreadable_ts_omits_the_line_and_a_hostile_slot_is_projected() {
        let bad = "{\"ts\":\"nope\",\"actor\":\"ae:seats:u\",\"action\":\"seat-compact\",\
                   \"run\":\"run-1\",\"target_slot\":\"w1\",\"summary\":\"dispatched w1\"}\n";
        assert!(audit_warning(bad.as_bytes(), ACTOR, NOW).is_empty());
        let slot = format!("\u{1b}[31m{}", "x".repeat(300));
        let seed = format!(
            "{{\"ts\":\"{TS}\",\"actor\":\"{ACTOR}\",\"action\":\"seat-compact\",\
             \"run\":\"run-1\",\"target_slot\":\"{slot}\",\"summary\":\"dispatched w1\"}}\n"
        );
        let lines = audit_warning(seed.as_bytes(), ACTOR, NOW);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("did not end"));
        assert!(!lines[0].contains('\u{1b}'), "no raw escape byte");
        assert!(lines[0].len() < 130, "one bounded line");
    }

    fn entry<'a>(
        slot: &'a str,
        tool: &'a str,
        gate: Gate,
        outcome: &'a Outcome,
        elapsed: i64,
        checkpoint: Option<&'a str>,
    ) -> SeatLine<'a> {
        SeatLine {
            slot,
            tool,
            gate,
            outcome,
            elapsed,
            earlier_checkpoint: checkpoint,
            observation: None,
        }
    }

    #[test]
    fn the_report_is_one_line_per_seat_then_the_hand_line() {
        let busy = Outcome::skipped(BUSY);
        let dispatched = outcome(SubmitState::Submitted);
        let staged = outcome(SubmitState::StillStaged);
        let hand = Outcome::skipped(INPUT_NOT_MODELLED);
        let idle = Observation::Idle;
        let timeout = Observation::unobserved(TIMEOUT);
        let guided = classify(GUIDED, InputModel::BorderDelimited, false);
        let bare = classify(BARE, InputModel::StyleDelimited, false);
        let unmodelled = classify(BARE, InputModel::Unmodelled, false);
        let rendered = report(&[
            entry("w1", "claude", guided, &busy, 12, None),
            entry("w2", "codex", bare, &dispatched, 3, None),
            entry("w3", "codex", bare, &staged, 61, Some(REQUEST)),
            entry("w4", "opencode", unmodelled, &hand, 2, None),
            SeatLine {
                observation: Some(&idle),
                ..entry("w5", "claude", guided, &dispatched, 40, None)
            },
            SeatLine {
                observation: Some(&timeout),
                ..entry("w6", "codex", bare, &dispatched, 600, None)
            },
        ]);
        assert_eq!(
            rendered,
            "skipped (busy) w1 (12s)\n\
             dispatched w2 (3s)\n\
             not dispatched (staged text) w3 (1m) — clear the composer of %3 before any send\n  \
             note: earlier checkpoint ae-20260917T120000Z-00000001\n\
             skipped (input not modelled) w4 (2s)\n\
             dispatched (observed idle) w5 (40s)\n\
             dispatched (unobserved: timeout) w6 (10m)\n\
             compact by hand: w4 (opencode: input not modelled)\n"
        );
    }

    const BEFORE: &str = "✻ Worked for 2s · done";
    const AFTER: &str = "⎿  Compacted (ctrl+o to see full summary)";

    fn frame(state: HarnessState, row: Option<&str>) -> FrameReading {
        FrameReading {
            state,
            current: row.map(ToOwned::to_owned),
        }
    }

    fn seen(state: HarnessState, row: &str) -> Sample {
        Sample::Frame(Some(frame(state, Some(row))))
    }

    #[test]
    fn only_two_consecutive_new_idle_frames_prove_idle() {
        let baseline = frame(HarnessState::Idle, Some(BEFORE));
        let mut watch = Watch::new(Some(baseline)).expect("a recognized baseline");
        for _ in 0..5 {
            let old = seen(HarnessState::Idle, BEFORE);
            assert_eq!(watch.feed(old), None, "the frame before the dispatch");
        }
        assert_eq!(watch.feed(seen(HarnessState::Idle, AFTER)), None, "one");
        for reset in [
            Sample::Blind,
            Sample::Frame(None),
            seen(HarnessState::Busy, "✶ Compacting… (3s)"),
            seen(HarnessState::Unknown, "an unfamiliar row"),
            seen(HarnessState::Idle, BEFORE),
        ] {
            let why = format!("{reset:?}");
            assert_eq!(watch.feed(reset), None, "{why}");
            let again = watch.feed(seen(HarnessState::Idle, AFTER));
            assert_eq!(again, None, "{why} restarted the pair");
        }
        let second = watch.feed(seen(HarnessState::Idle, AFTER));
        assert_eq!(second, Some(Observation::Idle));
    }

    #[test]
    fn a_rowless_baseline_needs_a_row_and_an_unrecognized_one_observes_nothing() {
        let empty = frame(HarnessState::Idle, None);
        let mut watch = Watch::new(Some(empty.clone())).expect("a recognized baseline");
        for _ in 0..3 {
            assert_eq!(watch.feed(Sample::Frame(Some(empty.clone()))), None);
        }
        assert_eq!(watch.feed(seen(HarnessState::Idle, AFTER)), None);
        let second = watch.feed(seen(HarnessState::Idle, AFTER));
        assert_eq!(second, Some(Observation::Idle));
        let unknown = Observation::unobserved(BASELINE_UNKNOWN);
        assert_eq!(Watch::new(None), Err(unknown));
    }

    #[test]
    fn a_guard_ends_the_watch_at_once_even_mid_pair() {
        for end in [
            Observation::unobserved(RELAUNCHED),
            Observation::unobserved(DEAD),
            Observation::unobserved(TIMEOUT),
            Observation::identity_gap(GapLeg::Mismatch),
        ] {
            let baseline = frame(HarnessState::Idle, Some(BEFORE));
            let mut watch = Watch::new(Some(baseline)).expect("a recognized baseline");
            assert_eq!(watch.feed(seen(HarnessState::Idle, AFTER)), None);
            assert_eq!(watch.feed(Sample::Ended(end.clone())), Some(end));
        }
    }

    fn observed(observation: &Observation, request: &str) -> String {
        observed_record(&ObservedRecord {
            ts: TS,
            actor: ACTOR,
            run: "run-1",
            request,
            slot: "w1",
            session: "s",
            observation,
        })
    }

    #[test]
    fn an_observed_record_binds_the_run_and_addresses_no_seat() {
        let idle = observed(&Observation::Idle, REQUEST);
        let gap = observed(&Observation::identity_gap(GapLeg::Unreadable), "");
        for (line, summary, reason) in [
            (&idle, "observed idle w1", None),
            (&gap, "unobserved w1", Some("identity gap: unreadable")),
        ] {
            let bytes = line.as_bytes();
            assert_eq!(extract(bytes, "action"), OBSERVED_ACTION.as_bytes());
            assert_eq!(extract(bytes, "summary"), summary.as_bytes());
            assert_eq!(extract(bytes, "run"), b"run-1");
            assert_eq!(extract(bytes, "target_slot"), b"w1");
            assert_eq!(extract(bytes, "target_session"), b"s");
            assert_eq!(line.contains("\"reason\""), reason.is_some(), "{line}");
            if let Some(reason) = reason {
                assert_eq!(extract(bytes, "reason"), reason.as_bytes());
            }
            let event = crate::events::Event::parse_line(line.trim_end()).expect("an event");
            assert!(event.target_identity().is_none(), "it addresses no seat");
        }
        assert_eq!(extract(idle.as_bytes(), "ref"), REQUEST.as_bytes());
        assert!(!gap.contains("\"ref\""));
    }

    #[test]
    fn an_observed_record_neither_warns_nor_silences_a_dispatch() {
        let dispatched = Outcome::from_verdict(Verdict::State(SubmitState::Submitted), "%1");
        let seat = record(ACTOR, &dispatched, REQUEST, 0);
        let start = run_record(TS, ACTOR, "run-1", RUN_START);
        let idle = observed(&Observation::Idle, REQUEST);
        assert_eq!(
            audit_warning(&audit(&[start, seat, idle]), ACTOR, NOW),
            ["note: w1 was dispatched 5m ago by a run that did not end"]
        );
    }

    #[test]
    fn every_record_derived_field_reaches_the_report_projected() {
        let slot = format!("\u{1b}[31m{}", "x".repeat(300));
        let reason = format!("r\u{7}eason {}", "y".repeat(300));
        let pane = "%\u{1b}3";
        let reference = format!("ref\u{1b}]0;t\u{7}{}", "z".repeat(300));
        let skipped = Outcome::Skipped { reason };
        let staged = Outcome::NotDispatched {
            reason: STAGED_TEXT.to_owned(),
            pane: Some(pane.to_owned()),
        };
        let bare = classify(BARE, InputModel::StyleDelimited, false);
        let guided = classify(GUIDED, InputModel::BorderDelimited, false);
        let rendered = report(&[
            entry(&slot, "codex", bare, &staged, 61, Some(&reference)),
            entry("w9", "claude", guided, &skipped, 2, None),
        ]);
        assert!(!rendered.contains('\u{1b}'), "no raw escape byte");
        assert!(!rendered.contains('\u{7}'), "no raw control byte");
        assert!(
            rendered.matches('?').count() >= 4,
            "the projection said each one"
        );
        assert!(rendered.lines().all(|line| line.len() < 200), "bounded");
    }
}
