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

/// The draft rows drawn in columns `from..to` of `buf`: each row that holds
/// at least ten digits and nothing else, as its digits and the column of its
/// last digit.
fn draft_rows(buf: &Buffer, from: u16, to: u16) -> Vec<(String, u16)> {
    (0..buf.area.height)
        .filter_map(|y| {
            let cells: Vec<(u16, &str)> = (from..to).map(|x| (x, buf[(x, y)].symbol())).collect();
            let digits: Vec<&(u16, &str)> = cells
                .iter()
                .filter(|(_, s)| s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty())
                .collect();
            let row: String = cells.iter().map(|(_, s)| *s).collect();
            let rest = row.trim().trim_start_matches(|c: char| !c.is_ascii_digit());
            (digits.len() >= 10 && rest.chars().all(|c| c.is_ascii_digit()))
                .then(|| (rest.to_owned(), digits.last().map_or(0, |(x, _)| *x)))
        })
        .collect()
}

/// A wrapped draft fills each full row up to its column's right margin of
/// two cells and never into it, and every character stays drawn.
fn assert_fills_to_the_margin(rows: &[(String, u16)], draft: &str, end: u16) {
    assert!(rows.len() > 1, "the draft wraps: {rows:#?}");
    let shown: String = rows.iter().map(|(digits, _)| digits.as_str()).collect();
    assert_eq!(shown, draft, "{rows:#?}");
    let (last, full) = rows.split_last().expect("rows");
    for (_, x) in full {
        assert_eq!(*x, end - 3, "a full row ends before the margin: {rows:#?}");
    }
    assert!(last.1 <= end - 3, "{rows:#?}");
}

/// The side panel's border column in `buf`.
fn border_of(buf: &Buffer) -> u16 {
    (0..buf.area.width)
        .rev()
        .find(|x| (2..buf.area.height - 5).all(|y| buf[(*x, y)].symbol() == "│"))
        .expect("the side panel's border")
}

/// A draft written in the main column beside a side panel wraps inside the
/// main column's own cells, up to the margin before the border.
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
    let border = border_of(&buf);
    assert_fills_to_the_margin(&draft_rows(&buf, 0, border), &draft, border);
}

/// `api` home and a running orchestrator under `root`, proven as the
/// fixture's incarnation, with the panel open on it and the keys given.
fn orchestrated(root: &Root, full: bool) -> (App, Reader) {
    let (mut app, reader) = housed(root);
    let orch = crate::orchestrator::ORCHESTRATOR_SESSION;
    session(root, orch, "");
    refold(
        &mut app,
        &reader,
        root,
        &[("api", Status::Running), (orch, Status::Running)],
    );
    app.orch.proof = loader::Proof::Proven(ID.to_owned());
    app.open_panel(full);
    assert!(app.panel_focused());
    (app, reader)
}

/// Enter in the panel that holds the keys writes the orchestrator.
fn panel_writes(app: &mut App) {
    assert!(take_keys(app, vec![(Key::Enter, Instant::now())]).is_some());
    assert_eq!(
        app.writer_name(),
        Some(crate::orchestrator::ORCHESTRATOR_SESSION),
        "the panel writes its orchestrator"
    );
}

/// The panel's draft is its own: the main column beside it never shows it.
#[test]
fn the_panel_draft_never_shows_in_the_main_column() {
    let root = Root::new("o7own");
    let (mut app, _reader) = orchestrated(&root, false);
    panel_writes(&mut app);
    press(&mut app, b"PANEL_ONLY_DRAFT");
    let buf = at(160, 45, &mut app);
    let border = border_of(&buf);
    let row = |from: u16, to: u16| -> String {
        (0..buf.area.height)
            .map(|y| (from..to).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(
        row(border, 160).contains("PANEL_ONLY_DRAFT"),
        "{}",
        text(&buf)
    );
    assert!(
        !row(0, border).contains("PANEL_ONLY_DRAFT"),
        "{}",
        text(&buf)
    );
}

/// A full panel's draft wraps to the whole frame it covers, up to its margin.
#[test]
fn a_full_panel_draft_wraps_to_the_frame_it_covers() {
    let root = Root::new("o7full");
    let (mut app, _reader) = orchestrated(&root, true);
    panel_writes(&mut app);
    let draft = "0123456789".repeat(40);
    press(&mut app, draft.as_bytes());
    let buf = at(160, 45, &mut app);
    assert_fills_to_the_margin(&draft_rows(&buf, 0, 160), &draft, 160);
}

/// Hiding the panel while it writes ends its entry and keeps its draft, by
/// the main column's button beside a side panel and by a full panel's own.
#[test]
fn hiding_a_writing_panel_ends_its_entry_and_keeps_its_draft() {
    for full in [false, true] {
        hide_while_writing(full);
    }
}

fn hide_while_writing(full: bool) {
    let root = Root::new("o7hide");
    let (mut app, _reader) = orchestrated(&root, full);
    panel_writes(&mut app);
    press(&mut app, b"KEPT_PANEL_DRAFT");
    let shown = framed(&mut app);
    let (row, column) = shown
        .lines()
        .enumerate()
        .find_map(|(row, line)| line.chars().position(|c| c == '\u{25c6}').map(|x| (row, x)))
        .expect("the orchestrator button");
    let click = |kind| Mouse {
        kind,
        column: u16::try_from(column).expect("column"),
        row: u16::try_from(row).expect("row"),
    };
    assert!(app.mouse(click(MouseKind::Click)));
    let _ = app.mouse(click(MouseKind::Release));
    assert!(app.orch.panel.is_none(), "the button hides the panel");
    assert!(
        !app.composing() && app.writer_name().is_none(),
        "no hidden writer (full: {full})"
    );
    let orch = crate::orchestrator::ORCHESTRATOR_SESSION;
    let kept = app.inputs.get(&(orch.to_owned(), ID.to_owned()));
    assert_eq!(kept.map(Input::draft).as_deref(), Some("KEPT_PANEL_DRAFT"));
}

/// `F` from the main column beside a side panel turns the panel full, and a
/// full panel holds the keys.
#[test]
fn f_from_the_main_column_turns_the_panel_full_with_the_keys() {
    let mut app = app(None);
    two(&mut app);
    opened(&mut app, false);
    let _ = framed(&mut app);
    app.focus_column(false);
    press(&mut app, b"F");
    assert!(app.orch.panel.as_ref().is_some_and(|panel| panel.full));
    assert!(app.panel_focused(), "a full panel holds the keys");
    assert!(text(&at(160, 45, &mut app)).contains("F side"));
}

/// A click that selects another row owes a redraw.
#[test]
fn a_click_that_selects_a_row_redraws() {
    let mut app = app(None);
    two(&mut app);
    let shown = framed(&mut app);
    let (row, column) = shown
        .lines()
        .enumerate()
        .find_map(|(row, line)| line.find("ops").map(|column| (row, column)))
        .expect("drawn ops row");
    let redraw = app.mouse(Mouse {
        kind: MouseKind::Click,
        column: u16::try_from(column).expect("frame column"),
        row: u16::try_from(row).expect("frame row"),
    });
    assert_eq!(app.model.selected(), Some("ops"));
    assert!(redraw, "the selection changed what is drawn");
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
