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
