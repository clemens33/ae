//! Frozen acceptance spec for the tmux 3.8 `tmux-menus` doctor warning.

use super::*;

fn facts_with_tmux(probe: crate::tmux_floor::Probe) -> Facts {
    let mut input = super::tests::facts();
    input.tmux_floor = probe;
    input
}

#[test]
fn tmux_38_server_and_executable_emit_tmux_menus_warn_without_failing_floor_or_exit() {
    use crate::tmux_floor::Probe;

    for probe in [
        Probe::Server("3.8".to_owned()),
        Probe::Server("3.8a".to_owned()),
        Probe::Server("next-3.8".to_owned()),
        Probe::Executable("3.8".to_owned()),
        Probe::Executable("3.8a".to_owned()),
    ] {
        let out = report(&facts_with_tmux(probe.clone()));
        let floor = out
            .rows()
            .iter()
            .find(|row| row.label == "tmux-floor")
            .unwrap_or_else(|| panic!("missing tmux-floor row for {probe:?}"));
        assert_eq!(
            floor.level,
            Level::Ok,
            "tmux-floor must stay OK for {probe:?}: {}",
            out.render()
        );

        let menus: Vec<&Row> = out
            .rows()
            .iter()
            .filter(|row| row.label == "tmux-menus")
            .collect();
        assert_eq!(
            menus.len(),
            1,
            "expected one tmux-menus row for {probe:?}: {}",
            out.render()
        );
        let row = menus[0];
        assert_eq!(row.level, Level::Warn, "tmux-menus must be WARN: {row:?}");

        assert!(
            row.detail.contains(probe.found()),
            "tmux-menus detail must name observed release {:?} for {probe:?}: {}",
            probe.found(),
            row.detail
        );
        assert!(
            row.detail.contains(probe.source()),
            "tmux-menus detail must name source {:?} for {probe:?}: {}",
            probe.source(),
            row.detail
        );

        let lower = row.detail.to_ascii_lowercase();
        assert!(
            (lower.contains("every client") || lower.contains("all clients"))
                && lower.contains("window"),
            "tmux-menus detail must state menus draw on every/all clients of their window: {}",
            row.detail
        );
        for required in [
            "cross-session",
            "right-click",
            "clicked window",
            "3.7b",
            "brew pin tmux",
        ] {
            assert!(
                lower.contains(required),
                "tmux-menus detail missing {required:?} for {probe:?}: {}",
                row.detail
            );
        }

        assert_eq!(
            out.failures(),
            0,
            "WARN must not fail doctor: {}",
            out.render()
        );
        assert_eq!(
            out.exit_code(),
            0,
            "WARN must keep exit code 0: {}",
            out.render()
        );
    }
}

#[test]
fn tmux_34_through_37b_and_unreadable_probes_emit_no_tmux_menus_row() {
    use crate::tmux_floor::Probe;

    let clean_versions = ["3.4", "3.4a", "3.5", "3.5a", "3.6", "3.6a", "3.7", "3.7b"];
    let mut probes = Vec::new();
    for version in clean_versions {
        probes.push(Probe::Server(version.to_owned()));
        probes.push(Probe::Executable(version.to_owned()));
    }
    probes.push(Probe::Silent);
    probes.push(Probe::Unreachable);

    for probe in probes {
        let out = report(&facts_with_tmux(probe.clone()));
        assert!(
            !out.rows().iter().any(|row| row.label == "tmux-menus"),
            "unexpected tmux-menus row for {probe:?}: {}",
            out.render()
        );
    }
}
