//! Baseline-compatible requirement pin. Install as a child of app::draw.
#[test]
fn s5_red_retention_text_describes_the_maintained_store_exactly() {
    let expected = "Counts retained records in this state root, from a store kept up to date as workers spawn. A purge removes its archive's counts at once; a deleted session or archive drops out at the next update.";
    let mut bodies = crate::app::settings::SettingsBodies::default();
    bodies.stats.view = Some(Box::new(crate::stats::fold(Vec::new(), None)));
    let text = super::settings_body(
        crate::app::model::SettingsTab::Statistics,
        &bodies,
        super::Paint::of(None),
        512,
        false,
    )
    .into_iter()
    .map(|(line, _)| line.trim().to_owned())
    .collect::<Vec<_>>()
    .join(" ");
    assert!(
        text.contains(expected),
        "required retention disclosure absent: {text}"
    );
    assert!(!text.contains("No store is kept"));
}
