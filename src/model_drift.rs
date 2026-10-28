//! Observed model drift: what a live seat's frame proves, and the durable rows
//! it earns.
//!
//! ONE observer decision, ONE guarded writer, two callers — the watchdog every
//! cycle (the fast, reporting path) and `ae stop` under its lifecycle lock (the
//! durable cut that must carry the property). Both hand the same decision to
//! [`crate::meta::rewrite_observed`].
//!
//! A tool whose live model ae cannot read (a pane grammar it does not model, or
//! an unmeasured store) proves nothing here: its seats see no rows and are
//! reported as drift-unknown, never as preserved.

use std::path::Path;

use crate::effort::Level;
use crate::harness_state::{HarnessIdentity, current_identity};
use crate::meta::{Launched, ObservedEdit, ObservedPair, Rows};
use crate::tool::{PinMatch, ToolKind};

/// The `ae list` drift cell for one seat.
///
/// TABLE only, like `own_work`: the JSON digest's key set is a published
/// contract, and this renders facts the meta already carries.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ModelDrift {
    /// Nothing to report: the model matches the profile's pin, or the seat has
    /// not been observed drifting.
    #[default]
    Quiet,
    /// A recorded observation: the seat ran a model its profile does not pin.
    Observed(String),
    /// ae cannot observe this tool's model at all. Rendered loudly so nobody
    /// reads silence as preservation.
    Unknown,
}

/// What one positively parsed frame and the profile's pin decide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// No model was proved: write nothing. A failed capture must never erase a
    /// retained observation.
    Silent,
    /// The frame's model satisfies the current pin under the tool's rule: any
    /// retained row is retired.
    Retire,
    /// The frame's model does not satisfy the pin, or the profile pins none:
    /// record it. Recording is not injection — whether an observation may be
    /// replayed into the flag on resume is the adapter's own capability
    /// ([`crate::tool::ModelSpec`]). A `None` pin is report-only, because the
    /// profile has no model flag to rewrite and ae never appends one.
    Record {
        /// The observed model.
        model: String,
        /// The profile's model flag value at observation, where it pins one.
        pin: Option<String>,
    },
}

/// Decide from one frame identity, the profile's model flag value, and the
/// tool's pin-match rule.
#[must_use]
pub fn decide(identity: &HarnessIdentity, pin: Option<&str>, pin_match: PinMatch) -> Decision {
    let Some(model) = identity.model.as_deref() else {
        return Decision::Silent;
    };
    match pin {
        Some(pin) if satisfies(pin, model, pin_match) => Decision::Retire,
        pin => Decision::Record {
            model: model.to_owned(),
            pin: pin.map(str::to_owned),
        },
    }
}

/// Whether a drawn model name satisfies a pin under the adapter's rule.
///
/// Published inside the crate because the FOLLOW lookup
/// ([`crate::launch_cmd::followed_pin_in`]) asks the same question of a
/// CONFIGURED pin that the observer asks of the seat's own — one rule, never a
/// second copy that could disagree with the drift mark it must match.
pub(crate) fn satisfies(pin: &str, model: &str, pin_match: PinMatch) -> bool {
    match pin_match {
        PinMatch::Exact => pin == model,
        PinMatch::FamilyVersion => family_version_equivalent(pin, model),
    }
}

/// One model name split into family words and version numbers, in order.
struct FamilyVersion {
    family: Vec<String>,
    version: Vec<String>,
}

/// Sort one token into its list: the vendor token and date stamps are not
/// evidence of anything.
fn push_token(token: &str, family: &mut Vec<String>, version: &mut Vec<String>) {
    if token.is_empty() || token == "claude" {
        return;
    }
    let digits = token.bytes().all(|byte| byte.is_ascii_digit());
    if digits && token.len() == 8 {
        return;
    }
    if digits {
        version.push(token.to_owned());
    } else {
        family.push(token.to_owned());
    }
}

/// Tokenise one model name for the family/version rule: lowercase, drop a
/// trailing parenthesised suffix (decoration on both sides), split on every
/// non-alphanumeric char and on each letter/digit boundary, drop the vendor
/// token `claude` and any all-digit token of length 8 (a date stamp).
fn tokenise(text: &str) -> FamilyVersion {
    let lowered = text.to_lowercase();
    let bare = match lowered.rfind('(') {
        Some(open) if lowered.ends_with(')') => &lowered[..open],
        _ => lowered.as_str(),
    };
    let mut family = Vec::new();
    let mut version = Vec::new();
    let mut token = String::new();
    let mut token_is_digit = false;
    let mut open = false;
    for ch in bare.chars() {
        if !ch.is_alphanumeric() {
            if open {
                push_token(&token, &mut family, &mut version);
                token.clear();
                open = false;
            }
            continue;
        }
        let digit = ch.is_ascii_digit();
        if open && digit != token_is_digit {
            push_token(&token, &mut family, &mut version);
            token.clear();
        }
        token_is_digit = digit;
        open = true;
        token.push(ch);
    }
    if open {
        push_token(&token, &mut family, &mut version);
    }
    FamilyVersion { family, version }
}

/// Whether a pin value names a VERSION as well as a family.
///
/// The follow lookup's tie-break: between two distinct pin values that both
/// satisfy one label, the one naming a version is the more specific answer
/// (`claude-opus-5` over a bare `opus`). Derived from the same tokeniser the
/// match itself uses, so the two cannot disagree.
pub(crate) fn pin_names_a_version(pin: &str) -> bool {
    !tokenise(pin).version.is_empty()
}

/// Whether a drawn model name satisfies a pin under the family/version rule.
///
/// The families must agree exactly; a pin with no version numbers accepts any
/// drawn version of that family, otherwise the versions must agree too. An
/// empty family on either side never satisfies — that fails toward Record.
fn family_version_equivalent(pin: &str, model: &str) -> bool {
    let pin = tokenise(pin);
    let model = tokenise(model);
    if pin.family.is_empty() || model.family.is_empty() {
        return false;
    }
    if pin.family != model.family {
        return false;
    }
    pin.version.is_empty() || pin.version == model.version
}

/// What a profile pins this cycle, each where its command proves exactly one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Pins {
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<Level>,
}

/// The pair decision over one proved frame (`model`, and `effort` where drawn)
/// against the start's launch baseline, the observed record and the profile's
/// pins. First match wins:
///
/// 1. the pane still shows what this start launched: no human choice — a
///    refused resume, a profile edited while running and an ae-owned effort
///    level all look like this, so nothing is recorded or re-pinned;
/// 2. a damaged baseline and a frame at the pins: undecidable, kept;
/// 3. the frame equals the record: never re-pinned (an edit since is the
///    resume's to retire), and a baseline goes because the pane left it;
/// 4. the frame satisfies the pins: the record retires (a damaged one stays),
///    and so does a baseline the pane has left;
/// 5. otherwise the frame is recorded under the pins, replacing even a damaged
///    record — new evidence, not cleanup.
///
/// A baseline that pins no model never proves equality, so effort alone never
/// holds a choice back.
pub(crate) fn decide_pair(
    (model, effort): (&str, Option<Level>),
    pins: &Pins,
    pin_match: PinMatch,
    pair: &Rows<ObservedPair>,
    launched: &Rows<Launched>,
) -> ObservedEdit {
    let model_at = |pin: Option<&str>| pin.is_some_and(|pin| satisfies(pin, model, pin_match));
    let effort_at = |pin: Option<Level>| pin.is_none() || effort.is_none() || effort == pin;
    let baseline = match launched {
        Rows::Valid(baseline) => Some(baseline),
        _ => None,
    };
    if baseline.is_some_and(|base| model_at(base.model.as_deref()) && effort_at(base.effort)) {
        return ObservedEdit::Keep;
    }
    let at_pins = model_at(pins.model.as_deref()) && effort_at(pins.effort);
    if at_pins && matches!(launched, Rows::Damaged(_)) {
        return ObservedEdit::Keep;
    }
    if let Rows::Valid(record) = pair
        && record.model == model
        && record.effort.map_or_else(
            || effort_at(pins.effort),
            |(drawn, _)| effort.is_none_or(|effort| effort == drawn),
        )
    {
        return if baseline.is_some() {
            ObservedEdit::DropLaunched
        } else {
            ObservedEdit::Keep
        };
    }
    // A frame drawing no effort never proves a return to a recorded one.
    let unproved = effort.is_none() && matches!(pair, Rows::Valid(r) if r.effort.is_some());
    if at_pins {
        return if unproved || matches!(pair, Rows::Damaged(_)) {
            ObservedEdit::Keep
        } else {
            ObservedEdit::Retire {
                drop_launched: baseline.is_some(),
            }
        };
    }
    ObservedEdit::Record {
        pair: ObservedPair {
            model: model.to_owned(),
            model_pin: pins.model.clone(),
            effort: effort.zip(pins.effort),
        },
        drop_launched: baseline.is_some(),
    }
}

/// Observe one captured pane and publish what it proves.
///
/// Best-effort by construction: the watchdog swallows the error (the next
/// cycle retries), and `ae stop` turns it into one warning line without ever
/// failing the stop. Decided once lock-free, and again under the lock only
/// when something would change.
///
/// # Errors
///
/// The writer's refusal line: a malformed value, a missing launch id, or a
/// guard that no longer holds.
pub(crate) fn observe(
    dir: &Path,
    seat: (&str, &str),
    tool: ToolKind,
    capture: &str,
    launch_id: &str,
    pins: &Pins,
) -> Result<(), String> {
    let (slot, agent) = seat;
    let identity = current_identity(capture, tool);
    let Some(model) = identity.model else {
        return Ok(());
    };
    let frame = (
        model.as_str(),
        identity.effort.as_deref().and_then(Level::parse),
    );
    let decide = |bytes: &[u8]| {
        decide_pair(
            frame,
            pins,
            tool.adapter().pin_match,
            &crate::meta::observed_pair(bytes, slot),
            &crate::meta::launched(bytes, slot),
        )
    };
    if crate::meta::read_bytes(dir).is_ok_and(|bytes| decide(&bytes) == ObservedEdit::Keep) {
        return Ok(());
    }
    crate::meta::rewrite_observed(dir, slot, agent, tool, launch_id, decide)
        .map_err(|why| why.cause().to_string())
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "fixtures build and inspect real directories; the boundary is about \
                  what PRODUCT code may reach"
    )]
    use super::{Decision, decide, family_version_equivalent};
    use crate::harness_state::HarnessIdentity;
    use crate::tool::PinMatch;

    fn frame(model: &str) -> HarnessIdentity {
        HarnessIdentity {
            model: Some(model.to_owned()),
            effort: None,
        }
    }

    #[test]
    fn a_missing_model_is_silent_never_a_retirement() {
        assert_eq!(
            decide(&HarnessIdentity::default(), Some("fable"), PinMatch::Exact),
            Decision::Silent
        );
    }

    #[test]
    fn a_frame_equal_to_the_pin_retires_and_anything_else_records() {
        assert_eq!(
            decide(&frame("fable"), Some("fable"), PinMatch::Exact),
            Decision::Retire
        );
        assert_eq!(
            decide(&frame("opus"), Some("fable"), PinMatch::Exact),
            Decision::Record {
                model: "opus".to_owned(),
                pin: Some("fable".to_owned())
            }
        );
        assert_eq!(
            decide(&frame("opus"), None, PinMatch::Exact),
            Decision::Record {
                model: "opus".to_owned(),
                pin: None
            }
        );
    }

    #[test]
    fn family_version_matches_pins_to_display_labels() {
        for (pin, drawn) in [
            ("fable", "Fable 5.1"),
            ("claude-opus-5", "Opus 5"),
            ("claude-opus-5", "Opus 5 (1M context)"),
            ("claude-opus-5-5", "Opus 5.5 (1M context)"),
            ("opus", "Opus 5"),
            ("claude-haiku-4-5-20251001", "Haiku 4.5"),
        ] {
            assert!(family_version_equivalent(pin, drawn), "{pin} vs {drawn}");
        }
        for (pin, drawn) in [
            ("claude-opus-5", "Opus 5.1"),
            ("claude-opus-5", "Opus 5.5 (1M context)"),
            ("fable", "Opus 5 (1M context)"),
            ("claude-opus-5", "Sonnet 5"),
            // Families compare exactly, never by prefix, in either direction.
            ("opus", "Opus X 5"),
            ("opus-x", "Opus 5"),
            ("", "Fable 5.1"),
            ("fable", ""),
            ("5", "Opus 5"),
            ("Opus 5", "5"),
        ] {
            assert!(!family_version_equivalent(pin, drawn), "{pin} vs {drawn}");
        }
    }

    #[test]
    fn family_version_direction_follows_the_pin() {
        // A versioned pin is symmetric: both sides name the same version.
        assert!(family_version_equivalent("Opus 5", "claude-opus-5"));
        assert!(family_version_equivalent(
            "Haiku 4.5",
            "claude-haiku-4-5-20251001"
        ));
        // A bare pin accepts any drawn version — but as the DRAWN side it
        // proves no version, so the reverse does not hold.
        assert!(family_version_equivalent("fable", "Fable 5.1"));
        assert!(!family_version_equivalent("Fable 5.1", "fable"));
        assert!(family_version_equivalent("opus", "Opus 5"));
        assert!(!family_version_equivalent("Opus 5", "opus"));
    }

    #[test]
    fn decide_retires_a_claude_pin_its_own_label_satisfies() {
        assert_eq!(
            decide(&frame("Fable 5.1"), Some("fable"), PinMatch::FamilyVersion),
            Decision::Retire
        );
        assert_eq!(
            decide(
                &frame("Opus 5 (1M context)"),
                Some("fable"),
                PinMatch::FamilyVersion
            ),
            Decision::Record {
                model: "Opus 5 (1M context)".to_owned(),
                pin: Some("fable".to_owned())
            }
        );
        assert_eq!(
            decide(
                &frame("Opus 5.5 (1M context)"),
                Some("claude-opus-5-5"),
                PinMatch::FamilyVersion
            ),
            Decision::Retire
        );
    }

    #[test]
    fn decide_with_exact_keeps_byte_equality_for_drawn_flag_values() {
        assert_eq!(
            decide(&frame("gpt-5.6-sol"), Some("gpt-5.6-sol"), PinMatch::Exact),
            Decision::Retire
        );
        assert_eq!(
            decide(&frame("gpt-6-astra"), Some("gpt-5.6-sol"), PinMatch::Exact),
            Decision::Record {
                model: "gpt-6-astra".to_owned(),
                pin: Some("gpt-5.6-sol".to_owned())
            }
        );
    }

    #[test]
    fn the_pair_table_takes_the_first_match() {
        use super::{Pins, decide_pair};
        use crate::effort::Level::{High, Max, Xhigh};
        use crate::meta::{Launched, ObservedEdit, ObservedPair, Rows};
        let pins = Pins {
            model: Some("fable".to_owned()),
            effort: Some(Xhigh),
        };
        let launched = |model: Option<&str>, effort| {
            Rows::Valid(Launched {
                start: "main:L1:7".to_owned(),
                model: model.map(str::to_owned),
                effort,
            })
        };
        let opus_high = ObservedPair {
            model: "Opus 5.5".to_owned(),
            model_pin: Some("fable".to_owned()),
            effort: Some((High, Xhigh)),
        };
        let record = |drop_launched| ObservedEdit::Record {
            pair: opus_high.clone(),
            drop_launched,
        };
        let legacy = Rows::Valid(ObservedPair {
            effort: None,
            ..opus_high.clone()
        });
        let retire = |drop_launched| ObservedEdit::Retire { drop_launched };
        let opus = ("Opus 5.5", Some(High));
        let fable = ("Fable 5.1", Some(Xhigh));
        let cases = [
            // 1. the pane still shows this start's launch: a refused or edited
            //    start, an ae-owned level.
            (
                opus,
                Rows::Absent,
                launched(Some("claude-opus-5-5"), Some(High)),
                ObservedEdit::Keep,
            ),
            // ...but a baseline with no model never proves equality.
            (opus, Rows::Absent, launched(None, Some(High)), record(true)),
            // 2. damaged baseline, frame at the pins: undecidable.
            (
                fable,
                Rows::Valid(opus_high.clone()),
                Rows::Damaged("invalid"),
                ObservedEdit::Keep,
            ),
            // 3. the frame is the record: never re-pinned; a left baseline goes.
            (
                opus,
                Rows::Valid(opus_high.clone()),
                launched(Some("fable"), Some(Xhigh)),
                ObservedEdit::DropLaunched,
            ),
            (
                opus,
                Rows::Valid(opus_high.clone()),
                Rows::Absent,
                ObservedEdit::Keep,
            ),
            // 4. back at both pins: retired with a left baseline; damage stays.
            (
                fable,
                Rows::Valid(opus_high.clone()),
                launched(Some("claude-opus-5-5"), Some(High)),
                retire(true),
            ),
            (
                fable,
                Rows::Damaged("partial"),
                Rows::Absent,
                ObservedEdit::Keep,
            ),
            // A frame drawing no effort proves no return to a recorded one;
            // a legacy model-only record still retires on the model.
            (
                ("Fable 5.1", None),
                Rows::Valid(opus_high.clone()),
                Rows::Absent,
                ObservedEdit::Keep,
            ),
            (("Fable 5.1", None), legacy, Rows::Absent, retire(false)),
            // 5. new evidence replaces even a damaged record.
            (
                opus,
                Rows::Damaged("duplicate"),
                Rows::Absent,
                record(false),
            ),
            (opus, Rows::Absent, Rows::Damaged("partial"), record(false)),
        ];
        for (frame, pair, baseline, want) in cases {
            let got = decide_pair(frame, &pins, PinMatch::FamilyVersion, &pair, &baseline);
            assert_eq!(got, want, "{frame:?} {pair:?} {baseline:?}");
        }
        // Another effort on the recorded model is a new choice.
        let edit = decide_pair(
            ("Opus 5.5", Some(Max)),
            &pins,
            PinMatch::FamilyVersion,
            &Rows::Valid(opus_high.clone()),
            &Rows::Absent,
        );
        assert!(matches!(edit, ObservedEdit::Record { .. }), "{edit:?}");
    }

    #[test]
    fn a_followed_pair_left_for_the_pins_and_chosen_again_is_recorded_anew() {
        let dir = std::env::temp_dir().join(format!("ae-drift-round-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        // A followed start: the baseline is the recorded pair, not the pins.
        std::fs::write(
            dir.join("meta"),
            "schema=2\nseat.main=lead\nprofile.main=base\nagent_bin.main=claude\nlaunch_id.main=L1\n\
             observed_model.main=Opus 5.5\nobserved_model_pin.main=fable\n\
             observed_effort.main=high\nobserved_effort_pin.main=xhigh\n\
             launched.main=main:L1:7\nlaunched_model.main=claude-opus-5-5\nlaunched_effort.main=high\n",
        )
        .expect("meta");
        let pins = super::Pins {
            model: Some("fable".to_owned()),
            effort: Some(crate::effort::Level::Xhigh),
        };
        let border = "─".repeat(64);
        let frame = |drawn: &str| {
            format!("{border}\n❯\n{border}\n  🧠 {drawn}  📁 ae\n  ⏵⏵ bypass permissions on\n")
        };
        let observe = |drawn: &str| {
            let capture = frame(drawn);
            super::observe(
                &dir,
                ("main", "lead"),
                crate::tool::ToolKind::Claude,
                &capture,
                "L1",
                &pins,
            )
            .expect("observe");
            std::fs::read_to_string(dir.join("meta")).expect("read")
        };
        let text = observe("Opus 5.5 (high)");
        assert!(text.contains("launched.main="), "the launch itself: {text}");
        let text = observe("Fable 5.1 (xhigh)");
        assert!(
            !text.contains("observed_") && !text.contains("launched"),
            "at the pins, the record and the left baseline go: {text}"
        );
        let text = observe("Opus 5.5 (high)");
        assert!(
            text.contains("observed_model.main=Opus 5.5\n")
                && text.contains("observed_effort.main=high\n"),
            "the same pair chosen again is a new choice: {text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_return_to_both_pins_retires_the_record_on_the_next_observation() {
        let live = include_str!("../tests/fixtures/harness-state/claude-idle-167x40.txt");
        let dir = std::env::temp_dir().join(format!("ae-drift-heal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        std::fs::write(
            dir.join("meta"),
            "schema=2\nseat.main=lead\nprofile.main=fable5\nagent_bin.main=claude\nlaunch_id.main=L1\nobserved_model.main=Opus 5.5\nobserved_model_pin.main=fable\n",
        )
        .expect("meta");
        let pins = super::Pins {
            model: Some("fable".to_owned()),
            effort: Some(crate::effort::Level::Xhigh),
        };
        super::observe(
            &dir,
            ("main", "lead"),
            crate::tool::ToolKind::Claude,
            live,
            "L1",
            &pins,
        )
        .expect("observe");
        let text = std::fs::read_to_string(dir.join("meta")).expect("read");
        assert!(!text.contains("observed_model"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
