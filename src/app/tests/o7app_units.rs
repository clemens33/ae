//! Unit cases for the orchestrator panel beside the frozen acceptance: the
//! list keys beside a side panel, the redraw a panel mouse event owes, the
//! size floor under an open panel, and the main column's draft width beside
//! a side panel. A child of `app::tests`, driven by hand without tmux.

use super::*;

const OID: &str = "0199c0de-2222-4890-abcd-ef0123456789";

/// Open the panel side by side (or `full`) on a proven orchestrator.
fn opened(app: &mut App, full: bool) {
    app.orch.proof = loader::Proof::Proven(OID.to_owned());
    app.open_panel(full);
    assert!(app.panel_focused(), "the panel opens with the keys");
}

fn press(app: &mut App, bytes: &[u8]) {
    assert!(
        take_keys(app, vec![(Key::Text(bytes.to_vec()), Instant::now())]).is_some(),
        "a list key never quits the app"
    );
}

fn at(width: u16, height: u16, app: &mut App) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    app.frame(&mut buf);
    buf
}

/// Beside a side panel that holds the keys, the list's own keys still move
/// the main column's selection; over a full panel they do nothing.
#[test]
fn list_keys_beside_a_side_panel_move_the_main_selection() {
    let mut app = app(None);
    two(&mut app);
    assert_eq!(app.model.selected(), Some("web"));
    opened(&mut app, false);
    let _ = framed(&mut app);
    press(&mut app, b"j");
    assert_eq!(app.model.selected(), Some("ops"), "j moves the main list");
    press(&mut app, b"1");
    assert_eq!(app.model.selected(), Some("web"), "a digit selects its row");
    assert!(app.panel_focused(), "the panel keeps the keys");
    app.close_panel();
    opened(&mut app, true);
    let _ = framed(&mut app);
    press(&mut app, b"j");
    assert_eq!(
        app.model.selected(),
        Some("web"),
        "j over a full panel is inert"
    );
}

/// A mouse event on the panel owes a redraw exactly when it changes what is
/// drawn: a press that gives the panel the keys does, a release that ends
/// nothing does not.
#[test]
fn a_panel_mouse_event_redraws_only_when_it_changes_the_frame() {
    let mut app = app(None);
    two(&mut app);
    opened(&mut app, false);
    let buf = framed(&mut app);
    let row = u16::try_from(buf.lines().count() / 2).expect("frame row");
    let mouse = |kind| Mouse {
        kind,
        column: 150,
        row,
    };
    app.focus_column(false);
    assert!(!app.panel_focused());
    assert!(
        !app.mouse(mouse(MouseKind::Release)),
        "a release that ends nothing redraws nothing"
    );
    assert!(!app.panel_focused());
    assert!(
        app.mouse(mouse(MouseKind::Click)),
        "a press that hands the panel the keys redraws"
    );
    assert!(app.panel_focused());
}

/// Below the app's size floor an open panel draws nothing over the floor
/// note: the frame is the one a closed panel leaves, in either dimension.
#[test]
fn an_open_panel_draws_nothing_below_the_size_floor() {
    for (width, height) in [(30, 20), (160, 6)] {
        let mut app = app(None);
        two(&mut app);
        let closed = at(width, height, &mut app);
        opened(&mut app, false);
        let open = at(width, height, &mut app);
        assert_eq!(
            text(&open),
            text(&closed),
            "{width}x{height}: the floor note stands alone"
        );
        assert!(text(&open).contains("ae app needs at least"));
    }
}

/// A draft written in the main column beside a side panel wraps inside the
/// main column's own cells: every character stays drawn left of the border.
#[test]
fn a_main_draft_beside_a_side_panel_wraps_inside_its_column() {
    let root = Root::new("o7width");
    let (mut app, _reader) = housed(&root);
    opened(&mut app, false);
    app.focus_column(false);
    let started = writing(&mut app);
    let draft = "0123456789".repeat(15);
    assert_eq!(
        app.compose(Key::Text(draft.clone().into_bytes()), started),
        Some(())
    );
    let buf = at(160, 45, &mut app);
    let border = (0..buf.area.width)
        .rev()
        .find(|x| (2..buf.area.height - 5).all(|y| buf[(*x, y)].symbol() == "│"))
        .expect("the side panel's border");
    let rows: Vec<String> = (0..buf.area.height)
        .map(|y| {
            (0..border)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .filter(|row| {
            let digits = row.trim().trim_start_matches(|c: char| !c.is_ascii_digit());
            digits.len() >= 10 && digits.chars().all(|c| c.is_ascii_digit())
        })
        .collect();
    let shown: String = rows
        .iter()
        .flat_map(|row| row.chars().filter(char::is_ascii_digit))
        .collect();
    assert!(rows.len() > 1, "the draft wraps: {rows:#?}\n{}", text(&buf));
    assert_eq!(shown, draft, "{rows:#?}");
}

/// Lead ruling ae-20261114T141913Z-163e6319 (2): in a frame too narrow for
/// side, the column that holds the keys covers the frame. A main column
/// writing beside the panel stays on screen as the frame narrows, the open
/// panel returns beside it as the frame widens, and `O` hands the covered
/// panel the keys, which then covers the frame in turn.
#[test]
fn a_frame_too_narrow_for_side_shows_the_column_that_holds_the_keys() {
    let root = Root::new("o7narrow");
    let (mut app, _reader) = housed(&root);
    opened(&mut app, false);
    app.focus_column(false);
    let started = writing(&mut app);
    let typed = b"WRITTEN_UNDER_NARROWING".to_vec();
    assert_eq!(app.compose(Key::Text(typed), started), Some(()));
    let side = text(&at(160, 45, &mut app));
    assert!(
        side.contains("WRITTEN_UNDER_NARROWING") && side.contains("F full"),
        "{side}"
    );
    let narrow = at(90, 20, &mut app);
    let shown = text(&narrow);
    assert!(app.composing(), "the main entry goes on");
    assert!(
        shown.contains("WRITTEN_UNDER_NARROWING") && !shown.contains("side needs"),
        "the writing main column covers the narrow frame: {shown}"
    );
    let cursor = app.layout.cursor.expect("the main composer's cursor");
    assert!(cursor.x < 90 && cursor.y < 20);
    let wide = text(&at(160, 45, &mut app));
    assert!(
        wide.contains("WRITTEN_UNDER_NARROWING") && wide.contains("F full"),
        "widened, the panel stands beside the main column again: {wide}"
    );
    let _ = at(90, 20, &mut app);
    assert!(take_keys(&mut app, vec![(Key::Escape, Instant::now())]).is_some());
    assert!(!app.composing(), "Esc leaves writing");
    assert!(!app.panel_focused());
    press(&mut app, b"O");
    assert!(
        app.orch.panel.is_some() && app.panel_focused(),
        "O hands the covered panel the keys"
    );
    let covered = text(&at(90, 20, &mut app));
    assert!(
        covered.contains("side needs") && !covered.contains("WRITTEN_UNDER_NARROWING"),
        "the panel that holds the keys covers the frame: {covered}"
    );
}
