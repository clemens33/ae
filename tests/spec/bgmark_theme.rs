// Include inside theme.rs's existing cfg(test) mod tests.
#[test]
fn bgmark_ae_width_table_and_private_tmux_option_padding_agree() {
    assert_eq!(crate::orchestrator::terminal_cells("◉"), 1);
    assert_eq!(crate::orchestrator::terminal_cells("&"), 1);
    let rig = crate::transport::quietwd_spec::Rig::new("bgmark-cell");
    for glyph in ["◉", "&"] {
        rig.must(&["set-option", "-t", "=current:", "@bgmark_cell", glyph]);
        let padded = rig.must(&[
            "display-message",
            "-p",
            "-t",
            "=current:",
            "X#{p4:@bgmark_cell}Y",
        ]);
        assert_eq!(padded.trim_end_matches('\n'), format!("X{glyph}   Y"));
    }
}

#[test]
fn bgmark_orchestrator_button_uses_eight_distinct_accents() {
    for palette in [Palette::DARCULA, Palette::NEUTRAL, Palette::WARM] {
        let look = Look {
            palette,
            ..Look::DEFAULT
        };
        let mut foregrounds = Vec::new();
        for mark in [
            Mark::Dead,
            Mark::NeedsYou,
            Mark::Stale,
            Mark::Working,
            Mark::Background,
            Mark::WaitingAgent,
            Mark::Done,
            Mark::Idle,
        ] {
            let row = FleetRow {
                name: "orchestrator".to_owned(),
                id: "$7".to_owned(),
                mark,
                current: false,
            };
            let strip = orchestrator_strip(&look, &row, None);
            let foreground = strip
                .split_once(" fg=")
                .expect("styled button")
                .1
                .split(' ')
                .next()
                .expect("foreground")
                .to_ascii_lowercase();
            assert!(
                !foregrounds.contains(&foreground),
                "{}: duplicate {mark:?} foreground in {strip}",
                palette.name
            );
            foregrounds.push(foreground);
        }
        assert_eq!(foregrounds.len(), 8);
    }
}
