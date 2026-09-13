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
use crate::tool::ToolKind;

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
    /// The frame's model equals the current pin: any retained row is retired.
    Retire,
    /// The frame's model differs from the pin, or the profile pins none:
    /// record it. A `None` pin means report-only — the profile has no model
    /// flag to rewrite, and ae never appends one.
    Record {
        /// The observed model.
        model: String,
        /// The profile's model flag value at observation, where it pins one.
        pin: Option<String>,
    },
}

/// Decide from one frame identity and the profile's model flag value.
#[must_use]
pub fn decide(identity: &HarnessIdentity, pin: Option<&str>) -> Decision {
    let Some(model) = identity.model.as_deref() else {
        return Decision::Silent;
    };
    match pin {
        Some(pin) if pin == model => Decision::Retire,
        pin => Decision::Record {
            model: model.to_owned(),
            pin: pin.map(str::to_owned),
        },
    }
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
    match decide(&current_identity(capture, tool), pin) {
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
    use super::{Decision, decide};
    use crate::harness_state::HarnessIdentity;

    fn frame(model: &str) -> HarnessIdentity {
        HarnessIdentity {
            model: Some(model.to_owned()),
            effort: None,
        }
    }

    #[test]
    fn a_missing_model_is_silent_never_a_retirement() {
        assert_eq!(
            decide(&HarnessIdentity::default(), Some("fable")),
            Decision::Silent
        );
    }

    #[test]
    fn a_frame_equal_to_the_pin_retires_and_anything_else_records() {
        assert_eq!(decide(&frame("fable"), Some("fable")), Decision::Retire);
        assert_eq!(
            decide(&frame("opus"), Some("fable")),
            Decision::Record {
                model: "opus".to_owned(),
                pin: Some("fable".to_owned())
            }
        );
        assert_eq!(
            decide(&frame("opus"), None),
            Decision::Record {
                model: "opus".to_owned(),
                pin: None
            }
        );
    }
}
