//! O8 outcome 3: a panel width follows the same restart handover as the
//! sidebar. The requirement keeps at least 40 columns in both side columns.
//! These tests cross the hostile wire parser, `Model::adopt`, `Model::view` and
//! the rendered layout; no test reads a newly introduced panel-width field.

use super::*;
use crate::app::handover::{self, Handover};
use crate::app::model::Columns;

fn legacy() -> String {
    format!(
        "v1|%1|{}|100|web|{ID}|agents|lead|31|18|0|9||0",
        crate::VERSION
    )
}

fn read(text: &str) -> Handover {
    handover::parse(text).expect("the canonical o8 handover must parse")
}

fn adopted(view: &handover::View) -> App {
    let mut app = app(None);
    two(&mut app);
    app.model.adopt(&app.fleet, view);
    app
}

fn width(app: &App, columns: u16) -> u16 {
    draw::panel_area(
        Rect::new(0, 0, columns, 45),
        app.model.split(),
        Columns {
            panel: Some((false, true)),
            ..Columns::default()
        },
    )
    .expect("the focused panel occupies a column")
    .width
}

/// GREEN at the o8 base: the new field must not invalidate a prior core's
/// 14-field handover or lose any prior preference.
#[test]
fn a_legacy_handover_keeps_its_prior_preferences() {
    let old = read(&legacy());
    let app = adopted(&old.view);
    let carried = app.model.view(ID).expect("web is selected");
    assert_eq!(carried, old.view, "all prior preferences survive adoption");
    assert!(old.admits("%1", crate::VERSION, 100));
    let mut written = old;
    written.view = carried;
    assert_eq!(
        written.render(),
        legacy(),
        "without a panel preference the writer keeps the legacy 14 fields"
    );
}

/// RED at the o8 base at the parser outcome, before any new API is needed.
#[test]
fn a_panel_width_crosses_adopt_view_render_and_a_second_app() {
    let text = format!("{}|56", legacy());
    let carried = handover::parse(&text);
    assert!(
        carried.is_some(),
        "o8 must accept the trailing panel-width field"
    );
    let carried = carried.expect("the asserted canonical o8 handover");
    let first = adopted(&carried.view);
    assert_eq!(
        width(&first, 160),
        56,
        "the layout consumes the adopted width"
    );
    let mut next = carried.clone();
    next.view = first.model.view(ID).expect("the selected web view");
    assert_eq!(next.render(), text, "the writer carries the panel width");
    let second = adopted(&read(&next.render()).view);
    assert_eq!(
        width(&second, 160),
        56,
        "a new app restores the panel width"
    );
    // The full fallback covers the main column: from where its draft starts
    // (two margins before the draft, two after) to the frame's end.
    let narrow = Rect::new(0, 0, 100, 45);
    let draft = u16::try_from(draw::draft_width(narrow, second.model.split())).expect("width");
    let start = 100 - 4 - draft;
    assert!(start > 0, "the sidebar stays drawn");
    assert_eq!(
        width(&second, 100),
        100 - start,
        "a narrow frame uses full fallback over the main column"
    );
    assert_eq!(
        width(&second, 160),
        56,
        "wide restores the saved side width"
    );
    assert_eq!(
        second.model.view(ID),
        Some(next.view),
        "a frame clamp never overwrites the stored preference"
    );
}

#[test]
fn an_empty_panel_field_preserves_the_legacy_default() {
    let old = adopted(&read(&legacy()).view);
    let text = format!("{}|", legacy());
    let carried = handover::parse(&text);
    assert!(
        carried.is_some(),
        "o8 must accept an empty trailing panel preference"
    );
    let new = adopted(&carried.expect("the asserted canonical o8 handover").view);
    assert_eq!(
        new.model.split(),
        old.model.split(),
        "an empty field chooses the legacy default"
    );
    assert_eq!(width(&new, 160), width(&old, 160));
}

#[test]
fn the_panel_field_is_canonical_bounded_and_clamped_only_at_layout() {
    // 88 = 160 minus the 31-column sidebar, its rule, and the 40-column main floor.
    for (stored, visible) in [("0", 40), ("40", 40), ("65535", 88)] {
        let text = format!("{}|{stored}", legacy());
        let carried = handover::parse(&text);
        assert!(
            carried.is_some(),
            "o8 must accept the bounded panel preference {stored}"
        );
        let carried = carried.expect("the asserted canonical o8 handover");
        let app = adopted(&carried.view);
        assert_eq!(width(&app, 160), visible, "layout clamp for {stored}");
        let mut written = carried;
        written.view = app.model.view(ID).expect("web view");
        assert_eq!(written.render(), text, "layout keeps the stored {stored}");
    }
    for hostile in [
        "-1", "+1", "01", " 40", "40 ", "65536", "1.0", "x", "56|extra",
    ] {
        let text = format!("{}|{hostile}", legacy());
        assert!(
            handover::parse(&text).is_none(),
            "the hostile panel preference {hostile:?} must be refused"
        );
    }
}
