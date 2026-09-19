//! Observed model drift: what a live seat's frame proves, and the durable rows
//! it earns.
//!
//! ONE observer decision, ONE guarded writer, two callers — the watchdog every
//! cycle (the fast, reporting path) and `ae stop` under its lifecycle lock (the
//! durable cut that must carry the property). Both hand the same decision to
//! [`crate::meta::record_observed_model`].
//!
//! A tool whose live model ae cannot read (a pane grammar it does not model, or
//! an unmeasured store) proves nothing here: its seats see no rows and are
//! reported as drift-unknown, never as preserved.

use std::path::Path;

use crate::harness_state::{HarnessIdentity, current_identity};
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

/// Observe one captured pane and publish what it proves.
///
/// Best-effort by construction: the watchdog swallows the error (the next
/// cycle retries), and `ae stop` turns it into one warning line without ever
/// failing the stop.
///
/// # Errors
///
/// The writer's refusal line: a malformed value, a missing launch id, or a
/// guard that no longer holds.
pub(crate) fn observe(
    dir: &Path,
    slot: &str,
    agent: &str,
    tool: ToolKind,
    capture: &str,
    launch_id: &str,
    pin: Option<&str>,
) -> Result<(), String> {
    match decide(
        &current_identity(capture, tool),
        pin,
        tool.adapter().pin_match,
    ) {
        Decision::Silent => Ok(()),
        Decision::Retire => {
            crate::meta::record_observed_model(dir, slot, agent, tool, launch_id, None)
                .map_err(|why| why.cause().to_string())
        }
        Decision::Record { model, pin } => crate::meta::record_observed_model(
            dir,
            slot,
            agent,
            tool,
            launch_id,
            Some((&model, pin.as_deref())),
        )
        .map_err(|why| why.cause().to_string()),
    }
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
            ("opus", "Opus 5"),
            ("claude-haiku-4-5-20251001", "Haiku 4.5"),
        ] {
            assert!(family_version_equivalent(pin, drawn), "{pin} vs {drawn}");
        }
        for (pin, drawn) in [
            ("claude-opus-5", "Opus 5.1"),
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
    fn a_stale_false_row_self_heals_on_the_next_observation() {
        let live = include_str!("../tests/fixtures/harness-state/claude-idle-167x40.txt");
        let dir = std::env::temp_dir().join(format!("ae-drift-heal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        std::fs::write(
            dir.join("meta"),
            "schema=2\nseat.main=lead\nprofile.main=fable5\nagent_bin.main=claude\nlaunch_id.main=L1\nobserved_model.main=Fable 5.1\nobserved_model_pin.main=fable\n",
        )
        .expect("meta");
        super::observe(
            &dir,
            "main",
            "lead",
            crate::tool::ToolKind::Claude,
            live,
            "L1",
            Some("fable"),
        )
        .expect("observe");
        let text = std::fs::read_to_string(dir.join("meta")).expect("read");
        assert!(!text.contains("observed_model"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
