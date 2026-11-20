//! Frozen wiring contract: waiting tools read the pane; other tools keep returning.
//! `/` is a directory, never a server socket. No live server is queried or killed.

use super::wait_for_agent_start;
use crate::{inventory::ServerId, meta::Selector, tool::ToolKind};

#[test]
fn real_wait_calls_its_pane_sensor_and_keeps_nonwaiting_tools_out() {
    let server = ServerId::Selected(Selector::Socket("/".into()));
    for (tool, reads) in [
        (ToolKind::Claude, 10),
        (ToolKind::Codex, 10),
        (ToolKind::OpenCode, 10),
        (ToolKind::Unknown, 0),
    ] {
        let ((), seen) = crate::transport::spawns_during(|| {
            wait_for_agent_start(&server, "%314159", tool, "ae");
        });
        assert_eq!(seen.len(), reads, "tool={tool:?}, observations={seen:?}");
        let expected = [
            "tmux",
            "-u",
            "-S",
            "/",
            "display-message",
            "-p",
            "-t",
            "%314159",
            "#{pane_pid} | #{pane_current_command}",
        ];
        for argv in seen {
            assert_eq!(argv, expected, "tool={tool:?}");
        }
    }
}
