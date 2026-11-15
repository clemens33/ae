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
    assert!(take_keys(&mut app, vec![(Key::Down, Instant::now())]).is_some());
    assert_eq!(
        app.model.selected(),
        Some("ops"),
        "Down steps the main list"
    );
    assert!(take_keys(&mut app, vec![(Key::Up, Instant::now())]).is_some());
    assert_eq!(app.model.selected(), Some("web"), "Up steps it back");
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
    assert!(take_keys(&mut app, vec![(Key::Down, Instant::now())]).is_some());
    assert_eq!(
        app.model.selected(),
        Some("web"),
        "Down over a full panel is inert"
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
    session(
        root,
        orch,
        &format!(
            "meta_agent=true\nlocal_config={}\nmain_pane=%7\n",
            root.0.join("orchestrator.config").display()
        ),
    );
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

/// A panel's draft wraps to the cells the panel covers, up to its margin:
/// the whole frame full, its own half side by side.
#[test]
fn a_panel_draft_wraps_to_the_cells_the_panel_covers() {
    for full in [true, false] {
        let root = Root::new("o7full");
        let (mut app, _reader) = orchestrated(&root, full);
        panel_writes(&mut app);
        let draft = "0123456789".repeat(if full { 40 } else { 20 });
        press(&mut app, draft.as_bytes());
        let buf = at(160, 45, &mut app);
        let from = if full { 0 } else { border_of(&buf) + 1 };
        assert_fills_to_the_margin(&draft_rows(&buf, from, 160), &draft, 160);
    }
}

/// The orchestrator button's cell in the frame `shown`.
fn button(shown: &str) -> Mouse {
    let (row, column) = shown
        .lines()
        .enumerate()
        .find_map(|(row, line)| line.chars().position(|c| c == '\u{25c6}').map(|x| (row, x)))
        .expect("the orchestrator button");
    Mouse {
        kind: MouseKind::Click,
        column: u16::try_from(column).expect("column"),
        row: u16::try_from(row).expect("row"),
    }
}

/// Hiding the panel beside a main column that writes leaves that entry on.
#[test]
fn hiding_the_panel_leaves_the_main_writer_writing() {
    let root = Root::new("o7keep");
    let (mut app, _reader) = orchestrated(&root, false);
    app.focus_column(false);
    let started = writing(&mut app);
    assert_eq!(
        app.compose(Key::Text(b"MAIN_GOES_ON".to_vec()), started),
        Some(())
    );
    let shown = framed(&mut app);
    assert!(app.mouse(button(&shown)));
    assert!(app.orch.panel.is_none(), "the button hides the panel");
    assert!(app.composing(), "the main entry goes on");
    assert_eq!(app.writer_name(), Some("api"));
}

/// A refused panel entry is HELD in the panel: Enter there tries the
/// orchestrator again, never the main column's session.
#[test]
fn a_held_panel_entry_retries_in_the_panel() {
    let root = Root::new("o7held");
    let (mut app, _reader) = orchestrated(&root, false);
    let orch = crate::orchestrator::ORCHESTRATOR_SESSION;
    let meta = root.0.join("sessions").join(orch).join("meta");
    std::fs::write(
        &meta,
        format!("schema=2\nsession_id={ID}\nlayout=lead-pair\n"),
    )
    .expect("meta");
    assert!(take_keys(&mut app, vec![(Key::Enter, Instant::now())]).is_some());
    assert!(
        app.held.is_some(),
        "a panel entry without its pair is refused"
    );
    assert!(app.writer_name().is_none());
    session(
        &root,
        orch,
        &format!(
            "meta_agent=true\nlocal_config={}\nmain_pane=%7\n",
            root.0.join("orchestrator.config").display()
        ),
    );
    assert!(take_keys(&mut app, vec![(Key::Enter, Instant::now())]).is_some());
    assert_eq!(
        app.writer_name(),
        Some(orch),
        "Enter retried the panel's entry"
    );
    assert!(app.held.is_none());
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

/// The reader open in the main column beside a side panel: the panel's
/// footer ends the frame with its gear, the frame's only one (the version
/// before it drops first on a narrow row).
#[test]
fn the_reader_beside_a_side_panel_leaves_the_panel_footer_its_gear() {
    let mut app = app(None);
    two(&mut app);
    opened(&mut app, false);
    app.focus_column(false);
    app.model.open_reader(1);
    let buf = at(160, 45, &mut app);
    let border = border_of(&buf);
    let gears: Vec<(u16, u16)> = (0..buf.area.height)
        .flat_map(|y| (0..160).map(move |x| (x, y)))
        .filter(|&(x, y)| buf[(x, y)].symbol() == "\u{2699}")
        .collect();
    assert_eq!(gears.len(), 1, "{}", text(&buf));
    let (x, y) = gears[0];
    assert!(
        x > border && y == buf.area.height - 1,
        "{x},{y}\n{}",
        text(&buf)
    );
}

/// A side request that cannot fit covers the frame with the panel, whose
/// header says why, at every width from the app's floor up.
#[test]
fn a_fallen_back_panel_says_why_at_every_width() {
    let mut app = app(None);
    two(&mut app);
    opened(&mut app, false);
    let mut fell = 0;
    for width in 40..160 {
        let shown = text(&at(width, 20, &mut app));
        let head = shown.lines().next().unwrap_or_default();
        if head.contains("F side") {
            fell += 1;
            assert!(head.contains("side needs"), "{width}: {head}");
        }
    }
    assert!(fell >= 40, "the narrow widths fall back: {fell}");
}

/// At exactly the app's floor, 40x8, an open panel draws: the floor is a
/// least size, not a size the panel refuses.
#[test]
fn an_open_panel_draws_at_exactly_the_size_floor() {
    let mut app = app(None);
    two(&mut app);
    opened(&mut app, false);
    let shown = text(&at(40, 8, &mut app));
    assert!(!shown.contains("ae app needs at least"), "{shown}");
    assert!(
        shown.contains("F side"),
        "the panel covers the floor frame: {shown}"
    );
}

/// The first column of `word` on row `y` of `buf`, searched from `from`.
fn cell_of(buf: &Buffer, word: &str, y: u16, from: u16) -> Option<u16> {
    let wide = u16::try_from(word.chars().count()).ok()?;
    (from..buf.area.width.saturating_sub(wide)).find(|&x| {
        word.chars()
            .zip(x..)
            .all(|(c, at)| buf[(at, y)].symbol() == c.to_string())
    })
}

/// With colour, the column that holds the keys has its name in the title
/// ink and the other column's name is dim; giving the keys away swaps them.
#[test]
fn the_focused_column_name_alone_wears_the_title_ink() {
    let root = Root::new("o7ink");
    let (mut app, _reader) = orchestrated(&root, false);
    for name in ["api", crate::orchestrator::ORCHESTRATOR_SESSION] {
        app.looks
            .insert(name.to_owned(), (Some(crate::theme::Look::DEFAULT), None));
    }
    let ink = |app: &mut App| {
        let buf = at(160, 45, app);
        let border = border_of(&buf);
        let main = cell_of(&buf, "api", 0, 0).expect("the main column's name");
        let panel = cell_of(&buf, "orchestrator", 0, border).expect("the panel's name");
        (buf[(main, 0)].fg, buf[(panel, 0)].fg)
    };
    let (main, panel) = ink(&mut app);
    assert_ne!(main, panel, "the panel holds the keys");
    app.focus_column(false);
    let (main_after, panel_after) = ink(&mut app);
    assert_eq!(
        (main_after, panel_after),
        (panel, main),
        "the inks swap with the keys"
    );
}

/// A press on the panel acts on the panel frame it was made on while the
/// bound keeps that frame, HISTORY frames back, and one made on a frame
/// the bound dropped is not taken, as in the main column.
#[test]
fn a_delayed_panel_press_acts_within_the_frame_bound_and_not_beyond() {
    for (frames, taken) in [
        (super::super::HISTORY, true),
        (super::super::HISTORY + 1, false),
    ] {
        let mut app = app(None);
        two(&mut app);
        opened(&mut app, false);
        let buf = at(160, 45, &mut app);
        let y = 0;
        let x = cell_of(&buf, "F full", y, border_of(&buf)).expect("the mode label");
        let origin = Instant::now();
        for _ in 0..frames {
            let _ = at(160, 45, &mut app);
        }
        let press = Mouse {
            kind: MouseKind::Click,
            column: x,
            row: y,
        };
        assert!(take_keys(&mut app, vec![(Key::Mouse(press), origin)]).is_some());
        let full = app.orch.panel.as_ref().is_some_and(|panel| panel.full);
        assert_eq!(full, taken, "{frames} frames back");
        let refused = app
            .flash
            .as_ref()
            .is_some_and(|(line, _)| line == "click not taken: its frame is gone");
        assert_eq!(refused, !taken, "{frames} frames back");
    }
}

/// A press on a full panel's own button hides the panel and nothing else: it
/// arms no selection in the main column that the panel uncovers.
#[test]
fn a_full_panel_button_press_hides_the_panel_and_arms_nothing() {
    let root = Root::new("o7fullbutton");
    let (mut app, _reader) = orchestrated(&root, true);
    let shown = framed(&mut app);
    assert!(app.mouse(button(&shown)));
    assert!(app.orch.panel.is_none(), "the button hides the panel");
    assert!(app.select.is_none(), "the press arms no selection");
}

/// A press the last drawn frame does not cover, as after the terminal grew
/// and before the redraw, leaves the keys where they are: it never gives a
/// side panel the keys, never takes them from one, and never hands them to
/// the main column a full panel covers.
#[test]
fn a_press_beyond_the_drawn_frame_leaves_the_keys_where_they_are() {
    for (full, focused) in [(false, false), (false, true), (true, true)] {
        let mut app = app(None);
        two(&mut app);
        opened(&mut app, full);
        let _ = at(160, 45, &mut app);
        app.focus_column(focused);
        for column in [160, 200, 250] {
            let _ = app.mouse(Mouse {
                kind: MouseKind::Click,
                column,
                row: 20,
            });
            assert_eq!(
                app.panel_focused(),
                focused,
                "full {full}, column {column}: the keys stay"
            );
        }
    }
}

/// Beside a side panel the frame shows one orchestrator button, the main
/// column's: the panel's footer draws none.
#[test]
fn a_side_panel_frame_shows_one_orchestrator_button() {
    let mut app = app(None);
    two(&mut app);
    opened(&mut app, false);
    let buf = at(160, 45, &mut app);
    let shown = text(&buf);
    assert_eq!(shown.matches('\u{25c6}').count(), 1, "{shown}");
    let border = border_of(&buf);
    let x = (0..160)
        .find(|&x| (0..45).any(|y| buf[(x, y)].symbol() == "\u{25c6}"))
        .expect("the orchestrator button");
    assert!(x < border, "the button is the main column's: {x}\n{shown}");
}

/// Opening the panel by its button while the main column writes hands the
/// panel the keys and ends the main entry, its draft kept: no column writes
/// without the keys.
#[test]
fn opening_the_panel_by_its_button_ends_the_main_entry_and_keeps_its_draft() {
    let root = Root::new("o7openwrite");
    let (mut app, _reader) = housed(&root);
    app.orch.proof = loader::Proof::Proven(OID.to_owned());
    let started = writing(&mut app);
    assert_eq!(
        app.compose(Key::Text(b"MAIN_DRAFT_KEPT".to_vec()), started),
        Some(())
    );
    let shown = framed(&mut app);
    assert!(app.mouse(button(&shown)));
    assert!(
        app.panel_focused(),
        "the button opens the panel with the keys"
    );
    assert!(
        !app.composing() && app.writer_name().is_none(),
        "the main entry ends"
    );
    let kept = app.inputs.get(&("api".to_owned(), app.id("api")));
    assert_eq!(kept.map(Input::draft).as_deref(), Some("MAIN_DRAFT_KEPT"));
}

/// A press on the panel's mode label flips the panel and nothing else: it
/// arms no selection, side to full and full to side.
#[test]
fn a_mode_label_press_flips_the_panel_and_arms_nothing() {
    for (full, label) in [(false, "F full"), (true, "F side")] {
        let mut app = app(None);
        two(&mut app);
        opened(&mut app, full);
        let buf = at(160, 45, &mut app);
        let from = if full { 0 } else { border_of(&buf) };
        let x = cell_of(&buf, label, 0, from).expect("the mode label");
        assert!(app.mouse(Mouse {
            kind: MouseKind::Click,
            column: x,
            row: 0,
        }));
        let flipped = app.orch.panel.as_ref().is_some_and(|panel| panel.full);
        assert_eq!(flipped, !full, "{label} flips the panel");
        assert!(app.select.is_none(), "{label}: the press arms no selection");
    }
}

/// Any press ends a HELD entry, as in the main column: a press on the mode
/// label of the panel that holds the keys ends the panel's refused entry.
#[test]
fn a_mode_label_press_ends_a_held_panel_entry() {
    for (full, label) in [(false, "F full"), (true, "F side")] {
        let root = Root::new("o7labelheld");
        let (mut app, _reader) = orchestrated(&root, full);
        let orch = crate::orchestrator::ORCHESTRATOR_SESSION;
        let meta = root.0.join("sessions").join(orch).join("meta");
        std::fs::write(
            &meta,
            format!("schema=2\nsession_id={ID}\nlayout=lead-pair\n"),
        )
        .expect("meta");
        assert!(take_keys(&mut app, vec![(Key::Enter, Instant::now())]).is_some());
        assert!(
            app.held.is_some(),
            "a panel entry without its pair is refused"
        );
        let buf = at(160, 45, &mut app);
        let from = if full { 0 } else { border_of(&buf) };
        let x = cell_of(&buf, label, 0, from).expect("the mode label");
        assert!(app.mouse(Mouse {
            kind: MouseKind::Click,
            column: x,
            row: 0,
        }));
        assert!(app.held.is_none(), "{label}: the press ends the held entry");
    }
}

/// The width a fallen-back panel names is the one it needs: there the panel
/// stands beside the main column, one column less it still falls back.
#[test]
fn a_fallen_back_panel_names_the_width_side_needs() {
    let mut app = app(None);
    two(&mut app);
    opened(&mut app, false);
    let head = |app: &mut App, width: u16| -> String {
        let shown = text(&at(width, 20, app));
        shown.lines().next().unwrap_or_default().to_owned()
    };
    let narrow = head(&mut app, 60);
    let need: u16 = narrow
        .split("side needs ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("the fallback names a width: {narrow}"));
    let at_need = head(&mut app, need);
    assert!(at_need.contains("F full"), "{need}: side fits: {at_need}");
    let below = head(&mut app, need - 1);
    assert!(below.contains("side needs"), "{}: {below}", need - 1);
}

/// A press on the side panel's gear opens the settings and leaves the keys
/// where they are: the gear acts on the app, not on the panel.
#[test]
fn the_side_panel_gear_opens_the_settings_and_moves_no_keys() {
    let mut app = app(None);
    two(&mut app);
    opened(&mut app, false);
    app.focus_column(false);
    let buf = at(160, 45, &mut app);
    let border = border_of(&buf);
    let (x, y) = (0..buf.area.height)
        .flat_map(|y| (border..160).map(move |x| (x, y)))
        .find(|&(x, y)| buf[(x, y)].symbol() == "\u{2699}")
        .expect("the side panel's gear");
    assert!(app.mouse(Mouse {
        kind: MouseKind::Click,
        column: x,
        row: y,
    }));
    assert!(app.model.settings_open(), "the gear opens the settings");
    assert!(!app.panel_focused(), "the gear moves no keys");
}

/// An incomplete read that omits the main column's selection moves it the
/// same way while the panel writes as while nothing writes: the panel's
/// entry never holds the main column's selection on a row the read dropped.
#[test]
fn an_incomplete_read_moves_the_main_selection_alike_while_the_panel_writes() {
    let orch = crate::orchestrator::ORCHESTRATOR_SESSION;
    let moved = |panel_writes_now: bool| -> Option<String> {
        let root = Root::new("o7partial");
        let (mut app, reader) = orchestrated(&root, false);
        session(&root, "web", "");
        let all = [
            ("api", Status::Running),
            ("web", Status::Running),
            (orch, Status::Running),
        ];
        refold(&mut app, &reader, &root, &all);
        let _ = app.model.select_name(&app.fleet, "web", 0);
        assert_eq!(app.model.selected(), Some("web"));
        if panel_writes_now {
            panel_writes(&mut app);
        }
        let mut read = read_of(&app, &[("api", ID), ("web", ID), (orch, ID)]);
        read.fleet.rows.retain(|row| row.name != "web");
        read.scanned = false;
        app.answer(Answer::Fleet(read));
        app.model.selected().map(str::to_owned)
    };
    let alone = moved(false);
    assert_ne!(
        alone.as_deref(),
        Some("web"),
        "the read moves the selection"
    );
    assert_eq!(moved(true), alone, "the panel's entry changes nothing");
}

/// `F` owes a redraw whenever it flips the panel, with no armed press to
/// end: side to full and full to side.
#[test]
fn f_that_flips_the_panel_redraws() {
    for full in [false, true] {
        let mut app = app(None);
        two(&mut app);
        opened(&mut app, full);
        let _ = framed(&mut app);
        assert!(app.armed.is_none());
        let redraw = take_keys(&mut app, vec![(Key::Text(b"F".to_vec()), Instant::now())]);
        let flipped = app.orch.panel.as_ref().is_some_and(|panel| panel.full);
        assert_eq!(flipped, !full, "F flips the panel");
        assert_eq!(redraw, Some(true), "the flip redraws (full: {full})");
    }
}

/// `o` and `v` over the panel that holds the keys do nothing: the panel
/// opens no session and arms no press.
#[test]
fn o_and_v_over_the_panel_are_inert() {
    for byte in *b"ov" {
        let mut app = app(None);
        two(&mut app);
        opened(&mut app, false);
        let _ = framed(&mut app);
        let redraw = take_keys(&mut app, vec![(Key::Text(vec![byte]), Instant::now())]);
        assert_eq!(redraw, Some(false), "{} redraws nothing", char::from(byte));
        assert!(app.armed.is_none(), "{} arms nothing", char::from(byte));
        assert!(app.panel_focused());
    }
}

/// A fallen-back panel's header keeps the orchestrator's name beside its
/// reason when the row has room for both.
#[test]
fn a_fallen_back_header_keeps_the_name_beside_its_reason() {
    let root = Root::new("o7fellname");
    let (mut app, _reader) = orchestrated(&root, false);
    let orch = crate::orchestrator::ORCHESTRATOR_SESSION;
    let shown = text(&at(60, 20, &mut app));
    let head = shown.lines().next().unwrap_or_default();
    assert!(head.contains("side needs"), "{head}");
    assert!(head.contains(orch), "{head}");
}

/// Hiding the panel clears only a flash the panel itself raised: a flash
/// the main column shows stays on.
#[test]
fn hiding_the_panel_keeps_the_main_columns_flash() {
    let mut app = app(None);
    two(&mut app);
    opened(&mut app, false);
    let _ = framed(&mut app);
    app.flash = Some(("MAIN_NOTE".to_owned(), Instant::now()));
    let _ = framed(&mut app);
    assert!(take_keys(&mut app, vec![(Key::Escape, Instant::now())]).is_some());
    assert!(app.orch.panel.is_none(), "Esc hides the panel");
    assert_eq!(
        app.flash.as_ref().map(|(line, _)| line.as_str()),
        Some("MAIN_NOTE"),
        "the main column's flash stays"
    );
}

/// `!` that moves the selection to the next entry that needs the human owes
/// a redraw.
#[test]
fn a_bang_that_moves_the_selection_redraws() {
    let mut app = app(None);
    two(&mut app);
    let _ = framed(&mut app);
    assert_eq!(app.model.selected(), Some("web"));
    if let Some(row) = app.fleet.rows.iter_mut().find(|row| row.name == "ops") {
        row.needy = true;
    }
    let redraw = take_keys(&mut app, vec![(Key::Text(b"!".to_vec()), Instant::now())]);
    assert_eq!(app.model.selected(), Some("ops"), "! selects the needy row");
    assert_eq!(redraw, Some(true), "the move redraws");
}

/// Beside the sidebar a side panel takes half of the cells right of the
/// sidebar's rule, rounded down, and the main column the rest.
#[test]
fn a_side_panel_takes_half_the_cells_right_of_the_sidebar() {
    let mut app = app(None);
    two(&mut app);
    opened(&mut app, false);
    for width in [160_u16, 161] {
        let buf = at(width, 45, &mut app);
        let rules: Vec<u16> = (0..width)
            .filter(|x| (2..buf.area.height - 5).all(|y| buf[(*x, y)].symbol() == "│"))
            .collect();
        let [rule, border] = rules[..] else {
            panic!(
                "{width}: the sidebar rule and the border: {rules:?}\n{}",
                text(&buf)
            );
        };
        assert_eq!(
            width - border,
            (width - (rule + 1)) / 2,
            "{width}: {rules:?}"
        );
    }
}

/// A panel just opened has read nothing of its orchestrator's lane yet, and
/// says so: its chat shows one `loading` row until the first read lands.
#[test]
fn a_panel_just_opened_says_loading_until_its_lane_is_read() {
    let root = Root::new("o7loading");
    let (mut app, _reader) = orchestrated(&root, false);
    let buf = at(160, 45, &mut app);
    let border = border_of(&buf);
    let panel: Vec<String> = (0..buf.area.height)
        .map(|y| {
            (border + 1..160)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect();
    assert!(
        panel.iter().any(|row| row.trim() == "loading"),
        "{}",
        text(&buf)
    );
}

/// `r` in the panel with no turn to read opens no reader and leaves the
/// panel as it stands: the keys stay, a full panel stays full.
#[test]
fn r_with_nothing_to_read_leaves_the_panel_as_it_stands() {
    for full in [false, true] {
        let mut app = app(None);
        two(&mut app);
        opened(&mut app, full);
        let _ = framed(&mut app);
        let _ = take_keys(&mut app, vec![(Key::Text(b"r".to_vec()), Instant::now())]);
        assert!(!app.model.reader_open(), "nothing to read (full: {full})");
        assert!(app.panel_focused(), "the keys stay (full: {full})");
        let stays = app.orch.panel.as_ref().is_some_and(|panel| panel.full);
        assert_eq!(stays, full, "the panel keeps its mode");
    }
}

/// Only a press is refused when the frame it was made on is gone: a wheel
/// notch on the panel, however late, acts on the frame on screen, as in the
/// main column.
#[test]
fn a_late_wheel_on_the_panel_is_never_refused() {
    let mut app = app(None);
    two(&mut app);
    opened(&mut app, false);
    let buf = at(160, 45, &mut app);
    let x = border_of(&buf) + 10;
    let origin = Instant::now();
    for _ in 0..=super::super::HISTORY {
        let _ = at(160, 45, &mut app);
    }
    let wheel = Mouse {
        kind: MouseKind::WheelDown,
        column: x,
        row: 20,
    };
    assert!(take_keys(&mut app, vec![(Key::Mouse(wheel), origin)]).is_some());
    let refused = app
        .flash
        .as_ref()
        .is_some_and(|(line, _)| line == "click not taken: its frame is gone");
    assert!(!refused, "a wheel is never refused");
}

/// A drag in the panel keeps its selection across the frames it spans and
/// its release copies the panel's own text, side by side and full, as a drag
/// in the main column does.
#[test]
fn a_panel_drag_keeps_its_selection_across_a_redraw_and_copies() {
    for full in [false, true] {
        let root = Root::new("o7paneldrag");
        let (mut app, _reader) = orchestrated(&root, full);
        let orch = crate::orchestrator::ORCHESTRATOR_SESSION;
        app.answer(view(orch, ID, 100, "PANELWORDS alpha beta gamma delta"));
        app.answer(view("api", ID, 100, "MAINWORDS alpha beta gamma delta"));
        let buf = at(160, 45, &mut app);
        let (x, y) = (0..45)
            .find_map(|y| cell_of(&buf, "PANELWORDS", y, 0).map(|x| (x, y)))
            .unwrap_or_else(|| panic!("the panel's words: {}", text(&buf)));
        let event = |app: &mut App, kind, column| {
            let mouse = Mouse {
                kind,
                column,
                row: y,
            };
            take_keys(app, vec![(Key::Mouse(mouse), Instant::now())])
        };
        assert!(event(&mut app, MouseKind::Click, x).is_some());
        assert!(event(&mut app, MouseKind::Drag, x + 9).is_some());
        let _ = at(160, 45, &mut app);
        assert!(
            app.select.as_ref().is_some_and(|held| held.sel.moved()),
            "full {full}: the drag outlives the redraw: {:?}",
            app.flash
        );
        assert!(event(&mut app, MouseKind::Release, x + 9).is_some());
        let _ = at(160, 45, &mut app);
        let copied = app.select.as_ref().and_then(|held| match &held.stage {
            super::super::Stage::Ready(text) => Some(text.clone()),
            _ => None,
        });
        assert_eq!(copied.as_deref(), Some("PANELWORDS"), "full {full}");
    }
}

/// The main sidebar cell of `ops` in `app`'s closed frame at 160x45.
fn ops_cell(app: &mut App) -> Mouse {
    let closed = at(160, 45, app);
    let (column, row) = (0..45)
        .find_map(|y| cell_of(&closed, "ops", y, 0).map(|x| (x, y)))
        .expect("the ops row");
    Mouse {
        kind: MouseKind::Click,
        column,
        row,
    }
}

/// A press made on a full panel is pinned to the frame it was made on: after
/// `F` turned the panel side, after the panel closed and after it opened
/// again, it never acts on the main row that the full panel covered and
/// never takes the keys from the panel.
#[test]
fn a_full_panel_press_never_acts_on_the_main_row_it_covered() {
    let wrong: Vec<String> = ["F", "close", "reopen"]
        .into_iter()
        .filter_map(full_panel_press_after)
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// What a press made on a full panel did wrong when it arrived after
/// `after`, or `None`.
fn full_panel_press_after(after: &str) -> Option<String> {
    {
        let mut app = app(None);
        two(&mut app);
        let stale = ops_cell(&mut app);
        opened(&mut app, true);
        let _ = at(160, 45, &mut app);
        let origin = Instant::now();
        match after {
            "F" => press(&mut app, b"F"),
            "close" => {
                assert!(take_keys(&mut app, vec![(Key::Escape, Instant::now())]).is_some());
            }
            _ => {
                assert!(take_keys(&mut app, vec![(Key::Escape, Instant::now())]).is_some());
                let _ = at(160, 45, &mut app);
                press(&mut app, b"O");
            }
        }
        let _ = at(160, 45, &mut app);
        assert!(take_keys(&mut app, vec![(Key::Mouse(stale), origin)]).is_some());
        if app.model.selected() != Some("web") {
            return Some(format!(
                "after {after}: selected {:?}",
                app.model.selected()
            ));
        }
        if after != "close" && !app.panel_focused() {
            return Some(format!("after {after}: the panel lost the keys"));
        }
        None
    }
}

/// Below the app's size floor the floor note stands alone, so a press lands
/// on no column and moves no keys; at the floor a press on the panel's cells
/// gives the panel the keys.
#[test]
fn a_press_below_the_size_floor_moves_no_keys() {
    for (width, height, full, focused, after) in [
        (39, 20, true, true, true),
        (39, 20, false, true, true),
        (160, 7, false, false, false),
        (160, 8, false, false, true),
    ] {
        let mut app = app(None);
        two(&mut app);
        opened(&mut app, full);
        let _ = at(width, height, &mut app);
        app.focus_column(focused);
        let _ = at(width, height, &mut app);
        let _ = app.mouse(Mouse {
            kind: MouseKind::Click,
            column: width - 3,
            row: height / 2,
        });
        assert_eq!(
            app.panel_focused(),
            after,
            "{width}x{height}, full {full}, focused {focused}"
        );
    }
}

/// A main-column press from before the panel came to cover its cell, by
/// `O` then `F` from closed or by `F` from side, is not taken: the keys
/// stay with the covering panel, and what is typed next shows on screen as
/// the panel's, never as a hidden main draft.
#[test]
fn an_old_main_press_never_writes_under_the_panel_that_covers_it() {
    let wrong: Vec<String> = ["closed", "side"]
        .into_iter()
        .filter_map(old_main_press_from)
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// What a main-column press made on a `from` frame did wrong when it arrived
/// after the panel went full, or `None`.
fn old_main_press_from(from: &str) -> Option<String> {
    let root = Root::new(&format!("o7covered{from}"));
    let (mut app, _reader) = orchestrated(&root, false);
    let orch = crate::orchestrator::ORCHESTRATOR_SESSION;
    if from == "closed" {
        assert!(take_keys(&mut app, vec![(Key::Escape, Instant::now())]).is_some());
    } else {
        let _ = at(160, 45, &mut app);
        app.focus_column(false);
    }
    let _ = at(160, 45, &mut app);
    let stale = (20..45)
        .flat_map(|row| {
            (0..80).map(move |column| Mouse {
                kind: MouseKind::Click,
                column,
                row,
            })
        })
        .find(|mouse| app.layout.in_chat(*mouse))
        .expect("a main chat cell");
    let origin = Instant::now();
    if from == "closed" {
        press(&mut app, b"O");
        let _ = at(160, 45, &mut app);
    }
    press(&mut app, b"F");
    let _ = at(160, 45, &mut app);
    assert!(take_keys(&mut app, vec![(Key::Mouse(stale), origin)]).is_some());
    let _ = at(160, 45, &mut app);
    let kept = app.panel_focused();
    assert!(take_keys(&mut app, vec![(Key::Enter, Instant::now())]).is_some());
    let pasted = vec![(Key::Pasted(b"COVEREDQX".to_vec()), Instant::now())];
    assert!(take_keys(&mut app, pasted).is_some());
    let shown = text(&at(160, 45, &mut app));
    let owners: Vec<String> = (app.inputs.iter())
        .filter(|(_, input)| input.draft().contains("COVEREDQX"))
        .map(|((name, _), _)| name.clone())
        .collect();
    let seen = shown.contains("COVEREDQX");
    let right = kept && app.writer_name() == Some(orch) && owners == [orch] && seen;
    (!right).then(|| {
        format!(
            "from {from}: panel kept the keys {kept}, writer {:?}, draft owners {owners:?}, on screen {seen}",
            app.writer_name()
        )
    })
}

/// One step of [`no_order_of_steps_lets_the_main_column_write_under_a_full_panel`].
#[derive(Clone, Copy, Debug)]
enum Step {
    O,
    F,
    Click,
    Enter,
    Paste,
}

/// How the steps of one run reach the app.
#[derive(Clone, Copy, Debug)]
enum Feed {
    /// All in one `take_keys` batch, one frame after.
    Batch,
    /// One `take_keys` each, no frame between them.
    Each,
    /// One `take_keys` each, a frame after every one.
    Drawn,
}

/// Every order of `O`, `F`, a main-column click, Enter and a paste, from
/// closed, from side with the main column's keys and from side with the
/// panel's, fed in one batch, step by step, or with a frame between steps:
/// whenever the panel is full, the panel holds the keys, the main column is
/// not the writer, and no step changed the main column's draft.
#[test]
fn no_order_of_steps_lets_the_main_column_write_under_a_full_panel() {
    let steps = [Step::O, Step::F, Step::Click, Step::Enter, Step::Paste];
    let mut orders = vec![Vec::new()];
    for _ in 0..steps.len() {
        orders = (orders.iter())
            .flat_map(|order: &Vec<Step>| {
                let left: Vec<Step> = (steps.iter())
                    .filter(|step| {
                        !order.iter().any(|done| {
                            std::mem::discriminant(*step) == std::mem::discriminant(done)
                        })
                    })
                    .copied()
                    .collect();
                left.into_iter().map(move |step| {
                    let mut next = order.clone();
                    next.push(step);
                    next
                })
            })
            .collect();
    }
    assert_eq!(orders.len(), 120);
    let mut wrong = Vec::new();
    let mut by = std::collections::BTreeMap::new();
    let mut runs = 0;
    for start in ["closed", "side main", "side panel"] {
        for feed in [Feed::Batch, Feed::Each, Feed::Drawn] {
            for order in &orders {
                runs += 1;
                if let Some(why) = full_panel_run(runs, start, feed, order) {
                    *by.entry(format!("{start}, {feed:?}")).or_insert(0) += 1;
                    wrong.push(why);
                }
            }
        }
    }
    assert_eq!(runs, 1080);
    assert!(
        wrong.is_empty(),
        "{} of {runs} runs wrong, by start and feed {by:?}: {:#?}",
        wrong.len(),
        &wrong[..wrong.len().min(6)]
    );
}

/// What one run did wrong while the panel was full, or `None`.
fn full_panel_run(run: usize, start: &str, feed: Feed, order: &[Step]) -> Option<String> {
    let root = Root::new(&format!("o7steps{run}"));
    let (mut app, _reader) = orchestrated(&root, false);
    let orch = crate::orchestrator::ORCHESTRATOR_SESSION;
    let api = ("api".to_owned(), app.id("api"));
    match start {
        "closed" => {
            assert!(take_keys(&mut app, vec![(Key::Escape, Instant::now())]).is_some());
        }
        "side main" => {
            let _ = at(160, 45, &mut app);
            app.focus_column(false);
        }
        _ => {}
    }
    let _ = at(160, 45, &mut app);
    let click = (20..45)
        .flat_map(|row| {
            (0..80).map(move |column| Mouse {
                kind: MouseKind::Click,
                column,
                row,
            })
        })
        .find(|mouse| app.layout.in_chat(*mouse))
        .expect("a main chat cell");
    let key = |step: Step| match step {
        Step::O => Key::Text(b"O".to_vec()),
        Step::F => Key::Text(b"F".to_vec()),
        Step::Click => Key::Mouse(click),
        Step::Enter => Key::Enter,
        Step::Paste => Key::Pasted(b"STEPQX".to_vec()),
    };
    let draft = |app: &App| app.inputs.get(&api).map(Input::draft).unwrap_or_default();
    let full = |app: &App| app.orch.panel.as_ref().is_some_and(|panel| panel.full);
    let check = |app: &App, before: &str, at: &str| -> Option<String> {
        if !full(app) {
            return None;
        }
        let writer = app.writer_name();
        let changed = draft(app) != before;
        (!app.panel_focused() || writer == Some("api") || changed).then(|| {
            format!(
                "{start}, {feed:?}, {order:?}, {at}: panel keys {}, writer {writer:?}, main draft changed {changed}",
                app.panel_focused()
            )
        })
    };
    match feed {
        Feed::Batch => {
            let batch = order
                .iter()
                .map(|step| (key(*step), Instant::now()))
                .collect();
            assert!(take_keys(&mut app, batch).is_some());
            let _ = at(160, 45, &mut app);
            let after = draft(&app);
            if let Some(why) = check(&app, &after, "after the batch") {
                return Some(why);
            }
            let pasted = vec![(Key::Pasted(b"LATERQX".to_vec()), Instant::now())];
            assert!(take_keys(&mut app, pasted).is_some());
            let shown = text(&at(160, 45, &mut app));
            let wrong = check(&app, &after, "a paste after the batch");
            let hidden =
                full(&app) && app.writer_name() == Some(orch) && !shown.contains("LATERQX");
            wrong.or_else(|| {
                hidden.then(|| {
                    format!("{start}, Batch, {order:?}: the panel's paste is not on screen")
                })
            })
        }
        Feed::Each | Feed::Drawn => {
            for (n, step) in order.iter().enumerate() {
                let before = draft(&app);
                assert!(take_keys(&mut app, vec![(key(*step), Instant::now())]).is_some());
                if matches!(feed, Feed::Drawn) {
                    let _ = at(160, 45, &mut app);
                }
                if let Some(why) = check(&app, &before, &format!("after step {n} {step:?}")) {
                    return Some(why);
                }
            }
            None
        }
    }
}

/// A main-column press from before a side panel opened over its cell is not
/// taken: the panel that covers the cell now keeps the keys, and the press
/// says its frame is gone.
#[test]
fn an_old_main_press_under_a_side_panel_moves_no_keys() {
    let root = Root::new("o7underside");
    let (mut app, _reader) = orchestrated(&root, false);
    assert!(take_keys(&mut app, vec![(Key::Escape, Instant::now())]).is_some());
    let _ = at(160, 45, &mut app);
    let stale = (20..45)
        .flat_map(|row| {
            (120..150).map(move |column| Mouse {
                kind: MouseKind::Click,
                column,
                row,
            })
        })
        .find(|mouse| app.layout.in_chat(*mouse))
        .expect("a main chat cell the side panel will cover");
    let origin = Instant::now();
    press(&mut app, b"O");
    let buf = at(160, 45, &mut app);
    assert!(
        border_of(&buf) < stale.column,
        "the side panel covers the cell"
    );
    assert!(take_keys(&mut app, vec![(Key::Mouse(stale), origin)]).is_some());
    assert!(app.panel_focused(), "the covering panel keeps the keys");
    assert_eq!(
        app.flash.as_ref().map(|(text, _)| text.as_str()),
        Some("click not taken: its frame is gone")
    );
}

/// A click on a main sidebar row read in the same batch as the `F` that
/// turns the panel full is not taken: the row it was aimed at is covered
/// before the click is handled, so the selection stays and the panel keeps
/// the keys.
#[test]
fn a_main_row_click_read_with_the_f_that_covers_it_is_not_taken() {
    let mut app = app(None);
    two(&mut app);
    let row = ops_cell(&mut app);
    opened(&mut app, false);
    let _ = at(160, 45, &mut app);
    app.focus_column(false);
    let _ = at(160, 45, &mut app);
    let batch = vec![
        (Key::Text(b"F".to_vec()), Instant::now()),
        (Key::Mouse(row), Instant::now()),
    ];
    assert!(take_keys(&mut app, batch).is_some());
    assert!(app.orch.panel.as_ref().is_some_and(|panel| panel.full));
    assert!(app.panel_focused(), "the full panel keeps the keys");
    assert_eq!(
        app.model.selected(),
        Some("web"),
        "the covered row is not selected"
    );
}
