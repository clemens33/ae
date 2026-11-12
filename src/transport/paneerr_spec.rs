//! Issue #334: a failed respawn must carry tmux's own reason.

use super::{quietwd_spec::Rig, run_tmux_op};
use crate::session_tmux::{Op, argv};

#[test]
fn missing_respawn_pane_reports_tmux_reason() {
    let rig = Rig::new("paneerr");
    let command = vec!["sleep 600".to_owned()];
    let (ok, reason) = run_tmux_op(&argv(
        &rig.server,
        &Op::RespawnPane {
            pane: "%2147483647",
            work_dir: "",
            command: &command,
        },
    ));
    assert!(!ok, "a missing pane cannot be respawned");
    assert!(
        reason.contains("can't find pane"),
        "tmux must name why respawn failed: {reason:?}"
    );
}
