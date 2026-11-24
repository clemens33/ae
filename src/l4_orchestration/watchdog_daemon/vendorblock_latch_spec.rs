//! Frozen seam: apply one pane's booked effects against a session directory.
//! The state root is that directory's grandparent, `<root>/sessions/<name>`.

#![allow(
    clippy::disallowed_methods,
    clippy::unwrap_used,
    reason = "the fixture applies booked effects in a scratch session directory"
)]

use super::{Acting, Cycle, Effect, Knobs, Observation, PaneState, SendHelper};

/// Apply `effects` the way the daemon does, in a session whose directory is
/// `meta_dir`. The block record lives in the state root, `meta_dir`'s
/// grandparent: `<root>/sessions/<name>` → `<root>/vendorblock/<client>.rec`.
pub(crate) fn apply_booked(
    meta_dir: &std::path::Path,
    effects: &[Effect],
    seen: &Observation,
    state: &mut PaneState,
) {
    std::fs::create_dir_all(meta_dir).unwrap();
    let helper = SendHelper::for_session(meta_dir);
    let server = crate::inventory::ServerId::Ambient;
    let auto_file = meta_dir.join("auto");
    let cycle = Cycle {
        knobs: Knobs::default(),
        meta_dir,
        helper: &helper,
        server: &server,
        session: "latch",
        goal: None,
        roster: Vec::new(),
        local_config: None,
        lead_pair: false,
        fleet_order: crate::theme::FleetOrder::EMPTY,
        meta_agent: false,
        launch_ids: Vec::new(),
        started: None,
        settled: None,
        auto: crate::autoreseat::settings_in(&auto_file, "[workspace]\nauto_reseat = off\n"),
        auto_trust: crate::config::AutoTrust::Off,
    };
    let acting = Acting {
        agent: "muse",
        slot: "spawned.0",
        pane: "%9",
        seen,
        events: &[],
        overview: None,
    };
    let mut err = Vec::new();
    for effect in effects {
        cycle
            .apply(effect, &acting, state, &mut err)
            .unwrap_or_else(|error| {
                panic!(
                    "apply {effect:?}: {error}; {}",
                    String::from_utf8_lossy(&err)
                )
            });
    }
}
