mod chrome_spec {
    include!("../../tests/it/app_chrome_spec.rs");
}

use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::loader::{self, Answer, Reader, ViewRead};
use super::upgrade::Blocker;
use super::{
    App, USAGE, browse, draw, facts_of, fleet, run, select, settings, settings_key, take_keys,
};
use crate::app::fleet::{Counts, Fleet, Row};
use crate::app::model::{Drag, Edge, Key as Browse, Model, SettingsTab};
use crate::attention::Reason;
use crate::console::input::{Effect, Input, Key, Mouse, MouseKind, Reading};
use crate::console::lane::{Item, Kind, Lane};
use crate::digest::{SessionEntry, Status};
use crate::listing::World;
use crate::render::Audience;
use crate::theme::Mark;
use crate::time::Timestamp;
use crate::tmux;
mod flight;
mod menu_acceptance;
mod menu_records;
mod menu_units;
mod o7app_spec;
mod o7app_units;
mod o8_handover_spec;
mod o8_identity_spec;
mod o8_mutation_spec;
mod o8_namesake_spec;
mod o8_overlay_spec;
mod o8_transition_spec;
mod o8_units;
mod o8_uuid_spec;
mod s2_exec_spec;

pub(in crate::app) const ID: &str = "0199c0de-1234-4890-abcd-ef0123456789";

/// A state root holding one lead-pair session, `api`, removed on drop
/// unless a private server it hosts may still be live. The flag is shared
/// with every fixture cleanup owner of a socket under this root: an owner
/// still registered, or an owner that could not prove its server ended,
/// keeps the root and its sockets reachable.
pub(in crate::app) struct Root(
    pub(in crate::app) PathBuf,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
);
impl Root {
    /// The retention flag a socket cleanup owner holds and sets on an
    /// unproven server end.
    pub(in crate::app) fn retention(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        std::sync::Arc::clone(&self.1)
    }

    pub(in crate::app) fn new(tag: &str) -> Self {
        // A state root this call created exclusively: the process id plus a
        // process-wide nonce names it, so no two calls share a path, even
        // with the same tag. A candidate that already exists (a stale root
        // of a reused process id) is skipped, never removed or reused.
        static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = loop {
            let nonce = NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let candidate =
                std::env::temp_dir().join(format!("ae-app-{}-{nonce}-{tag}", std::process::id()));
            match std::fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(why) if why.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(why) => panic!("state root {}: {why}", candidate.display()),
            }
        };
        let dir = root.join("sessions").join("api");
        std::fs::create_dir_all(&dir).expect("session dir");
        let meta = format!(
            "schema=2\nsession_id={ID}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\n"
        );
        std::fs::write(dir.join("meta"), meta).expect("meta");
        Self(root, std::sync::Arc::default())
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        use std::io::Write as _;
        // Exclusive access first: `get_mut` succeeds only when no other
        // owner holds the flag, and it synchronizes with every earlier
        // owner's final store and drop, so the read below cannot miss a
        // retention set by an owner that released concurrently.
        let reason = match std::sync::Arc::get_mut(&mut self.1) {
            None => "a server cleanup owner is still registered",
            Some(retained) => {
                if *retained.get_mut() {
                    "an owned server end was not proven"
                } else {
                    let _ = std::fs::remove_dir_all(&self.0);
                    return;
                }
            }
        };
        let _ = writeln!(
            std::io::stderr().lock(),
            "fixture: retaining root {}: {reason}",
            self.0.display()
        );
    }
}

/// A fixture belongs to the `Root` that created it: a later `Root` with the
/// same tag gets its own state root, and neither its construction nor its
/// drop touches the files of the first while the first is alive.
#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "the test reads back the fixture files it created"
)]
fn a_later_root_with_the_same_tag_leaves_the_first_root_alone() {
    let read = |path: &PathBuf| std::fs::read(path).map_err(|why| why.kind());
    let first = Root::new("owned-twice");
    let marker = first.0.join("marker");
    std::fs::write(&marker, "first fixture bytes").expect("marker");
    let meta_path = first.0.join("sessions").join("api").join("meta");
    let meta = read(&meta_path).expect("the first root's session metadata");
    assert!(
        !meta.is_empty(),
        "the first root must hold session metadata"
    );

    let second = Root::new("owned-twice");
    assert_eq!(
        read(&marker),
        Ok(b"first fixture bytes".to_vec()),
        "a later root erased the marker of the first root"
    );
    assert_eq!(
        read(&meta_path),
        Ok(meta.clone()),
        "a later root changed the session metadata of the first root"
    );
    assert_ne!(
        first.0, second.0,
        "a repeated tag must not share a state root"
    );

    drop(second);
    assert_eq!(
        read(&marker),
        Ok(b"first fixture bytes".to_vec()),
        "dropping a later root erased the marker of the first root"
    );
    assert_eq!(
        read(&meta_path),
        Ok(meta),
        "dropping a later root changed the session metadata of the first root"
    );
}

fn text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn one_row(home: Option<&str>) -> Fleet {
    Fleet {
        rows: vec![row("api", 1, home.is_some())],
        home: home.map(str::to_owned),
    }
}

fn row(name: &str, index: usize, home: bool) -> Row {
    Row {
        name: name.to_owned(),
        index,
        mark: Mark::Working,
        needy: false,
        unanswered: None,
        counts: Counts::Unknown,
        line2: fleet::Line2::NoGoal,
        home,
    }
}

fn said(body: &str, at: i64) -> Item {
    Item {
        micros: 1_759_500_000_000_000 + at * 1_000_000,
        kind: Kind::Said {
            who: "lead".to_owned(),
        },
        body: body.to_owned(),
        record: None,
    }
}

fn framed(app: &mut App) -> String {
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
    app.frame(&mut buf);
    text(&buf)
}

/// An app driven by hand, as if its first fleet read had landed with no
/// lane to wait for.
fn app(home: Option<&str>) -> App {
    let mut app = App::new(home.map(str::to_owned), None, None, None);
    (app.fleeted, app.loading) = (true, false);
    app
}

/// [`app`] on `root`'s home `api`, as its first fleet read finds it.
fn housed(root: &Root) -> (App, Reader) {
    let reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
    let mut app = app(Some("api"));
    refold(&mut app, &reader, root, &[("api", Status::Running)]);
    (app, reader)
}

/// `root`'s sessions `live` with their statuses, as `reader` folds them.
fn refold(app: &mut App, reader: &Reader, root: &Root, live: &[(&str, Status)]) {
    let dir = |name: &str| (name.to_owned(), root.0.join("sessions").join(name));
    let dirs = live.iter().map(|(name, _)| dir(name)).collect();
    let world = live.iter().map(|(name, status)| entry(name, *status, None));
    let world = World::new(Timestamp::now(), world.collect());
    let order = crate::theme::FleetOrder::EMPTY;
    let read = reader.fold(dirs, world, None, &order, Timestamp::now());
    app.answer(Answer::Fleet(read));
}

/// The draft the composer shows.
fn drafted(app: &App) -> String {
    app.shown_input().map(Input::draft).unwrap_or_default()
}

#[test]
fn clipboard_completion_alone_and_with_a_hint_owes_the_next_frame() {
    for success in [true, false] {
        for hint in [false, true] {
            let root = Root::new("image-redraw-pin");
            let (mut app, _reader) = housed(&root);
            let at = writing(&mut app);
            assert_eq!(app.compose(Key::Text(b"kept".to_vec()), at), Some(()));
            let _ = framed(&mut app);
            let path = root.0.join("sessions/api/messages/img-pin.png");
            if success {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, b"\x89PNG\r\n\x1a\nimage").unwrap();
            }
            let writer = app.writer.as_ref().unwrap();
            let (send, recv) = std::sync::mpsc::channel();
            app.clipboard = Some(((writer.key(), writer.entry), recv));
            assert!(!app.owed(), "pending read and no other debt owe no frame");
            assert!(app.clipboard.is_some());
            assert_eq!(drafted(&app), "kept");
            assert!(app.flash.is_none());
            app.again = hint;
            let answer = if success {
                Ok(path.clone())
            } else {
                Err("clipboard tool failed".to_owned())
            };
            send.send(answer).unwrap();
            let mut backend = super::backend::Ansi::new(Vec::new(), (160, 45));
            assert!(
                super::settle(&mut app, &mut backend).unwrap(),
                "completion owes a frame: success {success}, other hint {hint}"
            );
            assert!(app.clipboard.is_none());
            if success {
                assert_eq!(drafted(&app), "kept [Image #1]");
                assert!(app.flash.is_none());
            } else {
                assert_eq!(drafted(&app), "kept");
                assert_eq!(
                    app.flash.as_ref().map(|(line, _)| line.as_str()),
                    Some("paste not taken: clipboard tool failed")
                );
            }
            let frame = framed(&mut app);
            assert!(frame.contains(if success {
                "[Image #1]"
            } else {
                "paste not taken: clipboard tool failed"
            }));
        }
    }
}

#[test]
fn image_paste_browses_without_fetching_and_ignores_a_repeat() {
    let mut app = app(None);
    let _ = pressed(&mut app, b"\x16");
    assert!(app.clipboard.is_none());
    let (send, recv) = std::sync::mpsc::channel();
    app.clipboard = Some(((("old".to_owned(), "uuid".to_owned()), 7), recv));
    crate::clipboard::start(
        &mut app.clipboard,
        (("new".to_owned(), "uuid".to_owned()), 8),
        PathBuf::new(),
    );
    assert_eq!(app.clipboard.as_ref().unwrap().0.1, 7);
    send.send(Err("clipboard tool failed".to_owned())).unwrap();
    assert!(
        !app.image_ready(),
        "old entry must not touch a draft or flash"
    );
    assert!(app.clipboard.is_none());
    assert!(app.flash.is_none());
}

/// `root`'s home `api` recorded as incarnation `id`, lead pair lead + `peer`.
pub(in crate::app) fn meta(root: &Root, id: &str, peer: &str) {
    let meta = format!("session_id={id}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0={peer}\n");
    let dir = root.0.join("sessions").join("api");
    std::fs::write(dir.join("meta"), format!("schema=2\n{meta}")).expect("meta");
}

/// [`housed`]'s `app` writing home under its own entry: the instant
/// after it began.
fn writing(app: &mut App) -> Instant {
    app.take(Reading::Owner, Instant::now());
    app.write(true);
    assert!(app.composing(), "writing started");
    Instant::now()
}

/// A lane of one turn saying `body`, read as `id` at `seq`.
fn view(name: &str, id: &str, seq: u64, body: &str) -> Answer {
    Answer::View(ViewRead {
        name: name.to_owned(),
        id: id.to_owned(),
        seq,
        lane: Lane {
            items: vec![said(body, 0)],
            coverage: Vec::new(),
        },
        needs: None,
        roster: None,
    })
}

/// Two foreign sessions, `web` and `ops`, read by the fleet as `ID`.
fn two(app: &mut App) {
    app.fleet = Fleet {
        rows: vec![row("web", 1, false), row("ops", 2, false)],
        home: None,
    };
    for name in ["web", "ops"] {
        app.dirs.insert(name.to_owned(), PathBuf::from(name));
        app.ids.insert(name.to_owned(), ID.to_owned());
    }
    app.world = World::new(
        Timestamp::now(),
        vec![
            entry("web", Status::Running, None),
            entry("ops", Status::Running, None),
        ],
    );
    app.model = Model::new(&app.fleet);
}

/// A click belongs to the drawn session, never a replacement at its index.
#[test]
fn a_departed_mouse_target_never_selects_its_replacement() {
    let mut app = app(None);
    app.fleet = one_row(None);
    app.model = Model::new(&app.fleet);
    let shown = framed(&mut app);
    let (row, column) = shown
        .lines()
        .enumerate()
        .find_map(|(row, line)| line.find("api").map(|column| (row, column)))
        .expect("drawn api session");
    app.fleet.rows[0].name = "web".to_owned();
    let redraw = app.mouse(super::Mouse {
        kind: super::MouseKind::Click,
        column: u16::try_from(column).expect("frame column"),
        row: u16::try_from(row).expect("frame row"),
    });
    assert!(!redraw, "a departed target is a full no-op");
    assert_eq!(
        app.model.selected(),
        Some("api"),
        "the click never selected web"
    );
}

/// Motion on a dragged list rule the frame stopped drawing moves nothing,
/// though the sidebar beside it is still drawn; shown again, the rule
/// stands where it was last dragged.
#[test]
fn motion_on_a_list_rule_the_frame_hid_keeps_its_size() {
    let rule_row = |shown: &str| shown.lines().position(|line| line.starts_with('─'));
    let mut app = app(None);
    two(&mut app);
    let first = framed(&mut app);
    let row = u16::try_from(rule_row(&first).expect("list rule")).expect("frame row");
    let at = |kind, row| super::Mouse {
        kind,
        column: 2,
        row,
    };
    assert!(
        app.mouse(at(super::MouseKind::Click, row)),
        "a press grabs the rule"
    );
    assert!(
        app.mouse(at(super::MouseKind::Drag, row + 3)),
        "motion drags it"
    );
    let chosen = app.model.split();
    let dragged = rule_row(&framed(&mut app));
    assert_eq!(dragged, Some(usize::from(row + 3)), "the rule followed");
    let world = app.world.clone();
    app.world = World::new(Timestamp::now(), Vec::new());
    let hidden = framed(&mut app);
    assert_eq!(
        rule_row(&hidden),
        None,
        "no selected entry draws no list rule"
    );
    let sidebar = |shown: &str| shown.lines().next().and_then(|top| top.find('│'));
    assert!(sidebar(&first).is_some(), "a sidebar rule to keep");
    assert_eq!(
        sidebar(&hidden),
        sidebar(&first),
        "the sidebar rule still stands"
    );
    assert!(
        !app.mouse(at(super::MouseKind::Drag, row + 8)),
        "motion on the hidden rule is a no-op"
    );
    assert!(app.model.drag().is_some(), "the drag is still held");
    assert_eq!(app.model.split(), chosen, "the hidden rule kept its size");
    app.world = world;
    assert_eq!(
        rule_row(&framed(&mut app)),
        dragged,
        "shown again where it was dragged"
    );
}

/// A fleet read that empties the fleet, then one that fills it again,
/// keeps both chosen sizes and the drag still held: a read is not input.
/// The refilled fleet otherwise starts fresh, on its first row.
#[test]
fn an_emptied_then_refilled_fleet_keeps_its_sizes_and_drag() {
    let rule_row = |shown: &str| shown.lines().position(|line| line.starts_with('─'));
    let sidebar = |shown: &str| {
        (shown.lines().next()).and_then(|top| top.chars().position(|cell| cell == '│'))
    };
    let mouse = |kind, column: usize, row: usize| super::Mouse {
        kind,
        column: u16::try_from(column).expect("frame column"),
        row: u16::try_from(row).expect("frame row"),
    };
    let mut app = app(None);
    two(&mut app);
    let full = read_of(&app, &[("web", ID), ("ops", ID)]);
    let mut empty = read_of(&app, &[]);
    (empty.fleet, empty.world) = (Fleet::default(), World::new(Timestamp::now(), Vec::new()));
    let first = framed(&mut app);
    let column = sidebar(&first).expect("sidebar rule");
    assert!(app.mouse(mouse(super::MouseKind::Click, column, 1)));
    assert!(app.mouse(mouse(super::MouseKind::Drag, column + 6, 1)));
    assert!(app.mouse(mouse(super::MouseKind::Release, column + 6, 1)));
    let widened = framed(&mut app);
    let row = rule_row(&widened).expect("list rule");
    assert!(app.mouse(mouse(super::MouseKind::Click, 2, row)));
    assert!(app.mouse(mouse(super::MouseKind::Drag, 2, row + 3)));
    app.answer(Answer::Fleet(empty));
    assert_eq!(
        app.model.selected(),
        None,
        "the empty fleet selects nothing"
    );
    app.answer(Answer::Fleet(full));
    let refilled = framed(&mut app);
    assert_eq!(
        sidebar(&refilled),
        Some(column + 6),
        "the sidebar kept its width"
    );
    assert_eq!(rule_row(&refilled), Some(row + 3), "the list kept its rows");
    assert_eq!(
        app.model.selected(),
        Some("web"),
        "the first row is selected"
    );
    assert!(app.model.drag().is_some(), "the drag is still held");
    assert!(
        app.mouse(mouse(super::MouseKind::Drag, 2, row + 4)),
        "and still drags"
    );
    assert_eq!(rule_row(&framed(&mut app)), Some(row + 4));
}

/// The bottom hint row is inside chat and scrolls, like the composer above it.
#[test]
fn mouse_wheel_on_the_hint_row_scrolls() {
    let mut app = app(None);
    app.fleet = one_row(None);
    app.model = Model::new(&app.fleet);
    app.lane = Rc::new(Lane {
        items: (0..60).map(|at| said(&format!("turn {at}"), at)).collect(),
        coverage: Vec::new(),
    });
    let shown = framed(&mut app);
    assert!(shown.lines().last().expect("keys row").contains("? keys"));
    let mut mouse = super::Mouse {
        kind: super::MouseKind::WheelUp,
        column: 159,
        row: 44,
    };
    assert!(app.mouse(mouse), "hint-row wheel scrolls");
    assert_eq!(app.model.scroll_rows(app.layout.page_rows), 3);
    mouse.row -= 1;
    assert!(app.mouse(mouse), "the composer is inside chat");
    assert_eq!(app.model.scroll_rows(app.layout.page_rows), 6);
    mouse.row += 1;
    mouse.kind = super::MouseKind::WheelDown;
    assert!(app.mouse(mouse), "hint-row wheel down scrolls back");
    assert_eq!(app.model.scroll_rows(app.layout.page_rows), 3);
}

/// The whole drawn list takes the wheel; a notch past its end is a no-op.
#[test]
fn a_list_notch_over_its_last_card_scrolls_and_none_scrolls_past_the_end() {
    let mut app = app(None);
    let rows = (0..30).map(|at| row(&format!("n{at:02}"), at + 1, false));
    app.fleet = one_row(None);
    app.fleet.rows = rows.collect();
    app.model = Model::new(&app.fleet);
    let cards = |shown: &str| -> Vec<(usize, u8)> {
        let name = |w: &str| w.strip_prefix('n')?.parse().ok();
        let card = |(y, l): (usize, &str)| Some((y, l.split_whitespace().find_map(name)?));
        shown.lines().enumerate().skip(1).filter_map(card).collect()
    };
    let first = cards(&framed(&mut app));
    let row = u16::try_from(first.last().expect("drawn cards").0).expect("row");
    let notch = super::Mouse {
        kind: super::MouseKind::WheelDown,
        column: 2,
        row,
    };
    assert!(app.mouse(notch), "the last drawn card scrolls");
    assert_eq!(cards(&framed(&mut app))[0].1, 1, "by one session");
    for _ in first.len()..29 {
        assert!(app.mouse(notch), "one notch per hidden session");
        framed(&mut app);
    }
    let end = framed(&mut app);
    assert_eq!(cards(&end).last().map(|card| card.1), Some(29));
    assert!(!app.mouse(notch), "past the end is a no-op");
    assert_eq!(framed(&mut app), end, "and draws nothing new");
}

/// The Agents heading is a label, not a target: no cell of it hits.
#[test]
fn the_agents_heading_is_no_click_target() {
    let mut app = app(None);
    app.fleet = one_row(None);
    app.model = Model::new(&app.fleet);
    app.world = World::new(Timestamp::now(), vec![entry("api", Status::Running, None)]);
    app.facts.insert(
        "api".to_owned(),
        fleet::Facts::Seats {
            id: "$1".to_owned(),
            agents: Vec::new(),
        },
    );
    let shown = framed(&mut app);
    let (row, column) = (shown.lines().enumerate())
        .find_map(|(row, line)| {
            line.find("Agents 0")
                .map(|byte| (row, line[..byte].chars().count()))
        })
        .expect("drawn heading");
    for cell in 0.."Agents 0".len() {
        let mouse = super::Mouse {
            kind: super::MouseKind::Click,
            column: u16::try_from(column + cell).expect("frame column"),
            row: u16::try_from(row).expect("frame row"),
        };
        assert!(
            app.layout.hit(mouse).is_none(),
            "cell {cell} of the heading"
        );
    }
    assert!(!shown.contains("Overview"), "{shown}");
}

/// B2-F4: a frame bounds the scroll to the pages the lane holds — the
/// bound is the FIRST page count that shows the oldest turn, so one page
/// fewer hides it — and a lane that fits scrolls nowhere.
#[test]
fn a_frame_bounds_the_scroll_to_the_oldest_turn() {
    let mut app = app(None);
    app.fleet = one_row(None);
    app.model = Model::new(&app.fleet);
    let mut items = vec![said("oldest turn", 0)];
    items.extend((1..60).map(|at| said(&format!("turn {at}"), at)));
    app.lane = Rc::new(Lane {
        items,
        coverage: Vec::new(),
    });
    for _ in 0..80 {
        let _ = app.model.key(Browse::PageUp, &app.fleet, false);
    }
    let shown = framed(&mut app);
    let bound = app.model.pages();
    assert!(bound > 0 && bound < 80, "clamped to the lane: {bound}");
    assert!(shown.contains("oldest turn"), "the bound shows the oldest");
    let _ = app.model.key(Browse::PageDown, &app.fleet, false);
    assert!(
        !framed(&mut app).contains("oldest turn"),
        "one fewer hides it"
    );
    assert_eq!(app.model.pages(), bound - 1, "a scroll inside stays put");
    Rc::make_mut(&mut app.lane).items.truncate(2);
    let _ = framed(&mut app);
    assert_eq!(app.model.pages(), 0, "a lane that fits scrolls nowhere");
}

/// An ask without the lease, or after home was replaced under its name,
/// is refused BEFORE any ask, kept draft or body is written and says so
/// in the home lane; the unproven home ends the writing (R-B5/R-B7).
#[test]
fn an_unproven_ask_is_refused_and_writes_nothing() {
    let root = Root::new("ask");
    let dir = root.0.join("sessions").join("api");
    let (mut app, _reader) = housed(&root);
    app.take(Reading::Owner, Instant::now());
    let refused = app.begin(b"x", "lead", "x".to_owned());
    assert_eq!(refused.err(), Some("refused: not writing".to_owned()));
    let typed = writing(&mut app);
    meta(&root, &ID.replace("1234", "bbbb"), "colead");
    for key in [Key::Text(b"check the scopes".to_vec()), Key::Enter] {
        assert_eq!(app.compose(key, typed), Some(()));
    }
    assert!(app.deliver(), "the entered line was queued, then asked");
    let notice = &app.notices.last().expect("the outcome is said").1.body;
    assert_eq!(notice, "refused: the session was replaced or renamed");
    let written = crate::store::open(&dir).events_source();
    assert!(
        matches!(written, crate::store::SourceRead::Absent),
        "no record"
    );
    assert_eq!(super::submit::restore(&dir), super::submit::Draft::Nothing);
    assert_eq!(
        super::submit::recover(&app.recovery.record(&dir).expect("own lifetime")),
        super::submit::Draft::Nothing,
        "no own recovery bytes were published"
    );
    let body = std::fs::remove_dir(dir.join("messages")).map_err(|why| why.kind());
    assert_eq!(body, Err(std::io::ErrorKind::NotFound), "no body written");
    assert!(!app.composing(), "and no writing");
    assert_eq!(
        app.held.as_deref(),
        Some("the session was replaced or renamed")
    );
}

/// `/open` in an app with no tmux identity (no `$TMUX`, no `$TMUX_PANE`)
/// names that and moves nothing, once the home roster is read.
#[test]
fn open_outside_tmux_is_refused_by_name() {
    let root = Root::new("open");
    let (mut app, _reader) = housed(&root);
    app.fleet = one_row(Some("api"));
    app.model = Model::new(&app.fleet);
    let typed = writing(&mut app);
    let lead = crate::console::needs::SeatRef {
        slot: "main".to_owned(),
        name: "lead".to_owned(),
    };
    if let Answer::View(mut read) = view("api", ID, 1, "") {
        read.roster = Some(vec![lead]);
        app.answer(Answer::View(read));
    }
    for key in [Key::Text(b"/open lead".to_vec()), Key::Enter] {
        assert_eq!(app.compose(key, typed), Some(()));
    }
    let notice = &app.notices.last().expect("the refusal is said").1.body;
    assert_eq!(
        notice,
        "refused: /open lead: ae app is not running inside tmux; nothing selected"
    );
}

fn seat_ref(slot: &str, name: &str) -> crate::console::needs::SeatRef {
    crate::console::needs::SeatRef {
        slot: slot.to_owned(),
        name: name.to_owned(),
    }
}

/// A frame of `api` that drew `names`, none of them in a roster.
fn drawn_rows(names: &[&str], focused: &str) -> super::Drawn {
    super::Drawn {
        session: "api".to_owned(),
        uuid: ID.to_owned(),
        rows: names
            .iter()
            .map(|name| ((*name).to_owned(), None))
            .collect(),
        focused: Some(focused.to_owned()),
    }
}

/// `n` / `p` step the rows the LAST frame drew, wrapping both ways. With
/// no cursor yet `n` lands on the first row and `p` on the last; a frame
/// of another session moves nothing, and a name the frame did not draw is
/// only a preference.
#[test]
fn the_seat_cursor_steps_the_drawn_rows_and_wraps() {
    let root = Root::new("cursor");
    let (mut app, _reader) = housed(&root);
    let cursor = |app: &App| app.drawn.as_ref().and_then(|d| d.focused.clone());
    let names = ["lead", "colead", "scout"];
    app.drawn = Some(drawn_rows(&names, "lead"));
    assert_eq!(app.model.focus(), None, "no cursor until a key asks");
    assert!(app.move_focus(true));
    assert_eq!(
        app.model.focus(),
        Some("lead"),
        "the first n is the first row"
    );
    assert!(app.move_focus(true) && app.move_focus(true));
    assert_eq!(cursor(&app).as_deref(), Some("scout"));
    assert_eq!(app.model.focus(), Some("scout"));
    assert!(app.move_focus(true), "past the last row wraps");
    assert_eq!(cursor(&app).as_deref(), Some("lead"));
    assert!(app.move_focus(false), "before the first row wraps");
    assert_eq!(cursor(&app).as_deref(), Some("scout"));
    app.focus_seat(Some("ghost".to_owned()));
    assert_eq!(cursor(&app).as_deref(), Some("scout"), "not a drawn row");
    assert_eq!(app.model.focus(), Some("ghost"), "the preference is kept");
    assert!(app.move_focus(true), "an undrawn cursor starts again");
    assert_eq!(app.model.focus(), Some("lead"));
    app.model.set_focus(None);
    assert!(app.move_focus(false), "the first p is the last row");
    assert_eq!(app.model.focus(), Some("scout"));
    app.drawn = Some(super::Drawn {
        session: "web".to_owned(),
        ..drawn_rows(&names, "lead")
    });
    assert!(!app.move_focus(true), "a frame of another session");
    app.drawn = Some(super::Drawn {
        rows: Vec::new(),
        ..drawn_rows(&[], "lead")
    });
    assert!(!app.move_focus(false), "no seat drawn");
}

/// A frame freezes each drawn seat with the identity the roster of the
/// read it drew from held, and the incarnation of that read; `o` acts on
/// that and says why when it holds none.
#[test]
fn a_frame_freezes_the_drawn_seats_with_the_roster_it_drew_from() {
    let root = Root::new("freeze");
    let (mut app, _reader) = housed(&root);
    app.shown.insert(
        "api".to_owned(),
        super::Shown {
            id: "read-id".to_owned(),
            seq: 1,
            lane: Rc::default(),
            roster: Some(vec![
                seat_ref("main", "lead"),
                seat_ref("spawned.0", "scout"),
            ]),
        },
    );
    (app.layout.seats, app.layout.focused) = (
        vec!["lead".to_owned(), "ghost".to_owned()],
        Some("lead".to_owned()),
    );
    let mut drawn = app.freeze("api");
    assert_eq!(drawn.uuid, "read-id");
    assert_eq!(
        drawn.rows,
        [
            ("lead".to_owned(), Some(seat_ref("main", "lead"))),
            ("ghost".to_owned(), None)
        ]
    );
    assert_eq!(drawn.seat(), Ok(&seat_ref("main", "lead")));
    drawn.focused = Some("ghost".to_owned());
    assert_eq!(
        drawn.seat().map_err(|why| why.line("ghost")),
        Err(
            "refused: /open ghost: ghost is not in the roster read for api; nothing selected"
                .to_owned()
        )
    );
    drawn.focused = None;
    assert!(
        matches!(drawn.seat(), Err(crate::console::open::Refusal::NoSeat(why)) if why.contains("no seat is shown"))
    );
    assert_eq!(
        app.freeze("web").uuid,
        "",
        "a session not read has no incarnation"
    );
}

/// `o` acts only on the frame the human saw: none, or one of another
/// session, is refused by name; outside tmux it is refused before any read.
#[test]
fn open_acts_only_on_the_frame_that_was_drawn() {
    let root = Root::new("frozen-open");
    let (mut app, _reader) = housed(&root);
    let said = |app: &App| app.notices.last().expect("said").1.body.clone();
    app.open_focused();
    assert_eq!(
        said(&app),
        "refused: /open api: the view changed; nothing selected"
    );
    app.drawn = Some(super::Drawn {
        session: "web".to_owned(),
        ..drawn_rows(&["lead"], "lead")
    });
    app.open_focused();
    assert_eq!(
        said(&app),
        "refused: /open api: the view changed; nothing selected"
    );
    app.drawn = Some(drawn_rows(&["lead"], "lead"));
    app.open_focused();
    assert_eq!(
        said(&app),
        "refused: /open lead: ae app is not running inside tmux; nothing selected"
    );
}

// ---- mutation pins (pins-plan.md). Oracles: the fixture root's own
// ---- records, docs/app.md, docs/chat.md and the existing owners.

/// A second lead-pair session `name` under `root`, as `Root::new` writes `api`.
pub(in crate::app) fn session(root: &Root, name: &str, meta_tail: &str) -> PathBuf {
    let dir = root.0.join("sessions").join(name);
    std::fs::create_dir_all(&dir).expect("session dir");
    let meta = format!(
        "schema=2\nsession_id={ID}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\n{meta_tail}"
    );
    std::fs::write(dir.join("meta"), meta).expect("meta");
    dir
}

pub(in crate::app) fn entry(name: &str, status: Status, attention: Option<Reason>) -> SessionEntry {
    let mut entry = SessionEntry::new(name, status);
    entry.attention = attention;
    entry
}

/// R-B6: only an acquisition restores the kept line, and only into an
/// empty composer, with no merge; a demotion ends the writing, and
/// another window's lease never refuses the next entry (S2).
#[test]
fn only_an_acquisition_into_an_empty_composer_restores_the_kept_line() {
    let root = Root::new("take");
    let store = crate::store::open(&root.0.join("sessions").join("api"));
    let (mut app, _reader) = housed(&root);
    app.recovery
        .record(store.dir())
        .expect("own lifetime")
        .publish(b"kept")
        .expect("a kept draft");
    app.take(Reading::Owner, Instant::now());
    assert_eq!(drafted(&app), "", "a promotion restores nothing");
    let at = writing(&mut app);
    assert_eq!(app.compose(Key::Text(b" edited".to_vec()), at), Some(()));
    app.write(false);
    writing(&mut app);
    assert_eq!(drafted(&app), "kept edited", "restored once, no merge");
    app.take(Reading::NotOwner("gone".to_owned()), Instant::now());
    assert!(!app.composing(), "a demotion ends the writing");
    let _other = store.console_writer().expect("composing holds no lease");
    app.take(Reading::Owner, Instant::now());
    app.write(true);
    assert!(app.composing() && app.held.is_none(), "{:?}", app.held);
}

/// ^J is a line break in the draft and Enter everywhere else: it starts
/// writing from browse, and while writing it sends nothing.
#[test]
fn a_newline_key_breaks_the_draft_and_is_enter_outside_it() {
    let root = Root::new("newline");
    let (mut app, _reader) = housed(&root);
    let at = Instant::now();
    assert!(!app.composing());
    assert_eq!(take_keys(&mut app, vec![(Key::Newline, at)]), Some(true));
    assert!(app.composing(), "^J in browse starts writing like Enter");
    let typed = Instant::now();
    let keys = vec![
        (Key::Text(b"ab".to_vec()), typed),
        (Key::Newline, typed),
        (Key::Text(b"cd".to_vec()), typed),
    ];
    let _ = take_keys(&mut app, keys);
    assert_eq!(drafted(&app), "ab cd", "one draft, the break a row");
    assert!(app.queued.is_empty(), "nothing is sent");
}

/// R-B2/R-B3: the entry's own instant gates the keys: one read before it
/// is dropped and named once, one read at that instant is draft.
#[test]
fn keys_read_before_the_entry_are_dropped_and_named_once() {
    let root = Root::new("late");
    let (mut app, _reader) = housed(&root);
    let before = Instant::now();
    writing(&mut app);
    let taken = app.entered.as_ref().map(|entry| entry.at).expect("entered");
    for key in [Key::Text(b"early".to_vec()), Key::Enter] {
        assert_eq!(app.compose(key, before), Some(()));
    }
    assert_eq!(app.compose(Key::Text(b"late".to_vec()), taken), Some(()));
    let draft = drafted(&app);
    assert_eq!(draft, "late", "only keys after the entry: {draft}");
    let named = |item: &Item| item.body.contains("keys typed before writing started");
    assert_eq!(app.notices.iter().filter(|(_, i)| named(i)).count(), 1);
}

/// Ruling appuse-b R-B2: only keys read before writing started are
/// dropped. A press on the composer while writing starts nothing new: the
/// same entry stands, and a key read after it began is drafted.
#[test]
fn a_composer_press_while_writing_keeps_its_entry() {
    let root = Root::new("composer-press");
    let (mut app, _reader) = strip_app(&root);
    let typed = writing(&mut app);
    let taken = app.entered.as_ref().map(|it| it.at);
    let _ = framed(&mut app);
    let press = (0..45)
        .flat_map(|row| (0..160).map(move |column| (column, row)))
        .map(|(column, row)| super::Mouse {
            kind: super::MouseKind::Click,
            column,
            row,
        })
        .find(|press| matches!(app.layout.hit(*press), Some(super::draw::Hit::Compose)))
        .expect("drawn composer");
    let _ = app.click(press);
    assert_eq!(app.entered.as_ref().map(|it| it.at), taken, "same entry");
    assert!(take_keys(&mut app, vec![(Key::Text(b"k".to_vec()), typed)]).is_some());
    assert_eq!(drafted(&app), "k");
}

/// R-C1.3/R-C1.5: the address, the three drawn draft rows and the note
/// row are Compose; after Esc the rows above the compact composer are
/// lane, not ghost targets.
#[test]
fn every_grown_composer_row_and_hint_are_targets_until_browse_compacts_it() {
    let root = Root::new("c1-targets");
    let (mut app, _reader) = strip_app(&root);
    let at = writing(&mut app);
    let _ = app.compose(Key::Pasted(b"first\nsecond\nthird".to_vec()), at);
    let shown = framed(&mut app);
    assert!(
        shown
            .lines()
            .nth(41)
            .expect("first draft row")
            .contains("first")
    );
    let press = |row| super::Mouse {
        kind: super::MouseKind::Click,
        column: 70,
        row,
    };
    let taken = app.entered.as_ref().map(|entry| entry.at);
    for row in 40..=44 {
        assert!(
            matches!(app.layout.hit(press(row)), Some(super::draw::Hit::Compose)),
            "address, grown row or note {row}"
        );
        let _ = app.click(press(row));
        assert_eq!(app.entered.as_ref().map(|entry| entry.at), taken);
        assert_eq!(
            app.inputs.values().next().expect("input").draft(),
            "first second third"
        );
    }
    let _ = app.compose(Key::Escape, Instant::now());
    let shown = framed(&mut app);
    assert!(
        shown
            .lines()
            .nth(42)
            .expect("compact composer")
            .contains("to api")
    );
    assert!(!matches!(
        app.layout.hit(press(41)),
        Some(super::draw::Hit::Compose)
    ));
    let _ = app.click(press(41));
    assert!(!app.composing(), "old upper row cannot restart writing");
    for row in 42..=44 {
        assert!(matches!(
            app.layout.hit(press(row)),
            Some(super::draw::Hit::Compose)
        ));
    }
}

/// #23/#25: an entry is found by its own name, and a selected running
/// session draws its tabs (frame calm r18).
#[test]
fn an_entry_is_found_by_its_own_name() {
    let mut app = app(None);
    app.world = World::new(
        Timestamp::now(),
        vec![
            entry("api", Status::Running, None),
            entry("web", Status::Stopped, None),
        ],
    );
    assert_eq!(app.entry("web").map(|e| e.name.as_str()), Some("web"));
    assert_eq!(app.entry("api").map(|e| e.name.as_str()), Some("api"));
    assert!(app.entry("zzz").is_none());
    app.fleet = one_row(None);
    app.model = Model::new(&app.fleet);
    assert!(framed(&mut app).contains("Agents"));
}

/// D3 (docs/app.md Ownership): a selection is read-only and says why
/// before the fleet is read, while stopped, while its pair is unread or
/// unreadable, and while its pair differs from the one it was written with.
#[test]
fn an_unwritable_selection_is_read_only_and_says_why() {
    let root = Root::new("readonly");
    let (mut app, _reader) = housed(&root);
    let why = |app: &App| app.writable("api").err().unwrap_or_default();
    assert_eq!(why(&app), "", "GUARD a running read session is writable");
    app.world = World::new(Timestamp::now(), vec![entry("api", Status::Stopped, None)]);
    app.show();
    let text = framed(&mut app);
    assert!(text.contains("read-only · api is stopped"), "{text}");
    assert!(!text.contains("to api"), "{text}");
    app.world = World::new(Timestamp::now(), vec![entry("api", Status::Running, None)]);
    let unreadable = Err("the meta names no single main seat".to_owned());
    let read = app.pairs.insert("api".to_owned(), unreadable);
    assert_eq!(why(&app), "the meta names no single main seat");
    app.pairs.clear();
    assert_eq!(why(&app), "the lead pair is not read yet");
    app.pairs.insert("api".to_owned(), read.expect("pair"));
    app.bound
        .insert(("api".to_owned(), ID.to_owned()), Vec::new());
    assert!(why(&app).contains("restart"), "{}", why(&app));
    app.fleeted = false;
    assert_eq!(why(&app), "the fleet is not read yet");
}

/// B2: a session that is not home is written under its own entry and
/// input, under its own bare address, holding no lease (S2); only a read
/// proving its stop ends it.
#[test]
fn a_selection_that_is_not_home_is_written_under_its_own_entry() {
    let root = Root::new("selected");
    let (mut app, reader) = housed(&root);
    let web = session(&root, "web", "");
    let api = root.0.join("sessions").join("api");
    let fleet = |web| [("api", Status::Running), ("web", web)];
    refold(&mut app, &reader, &root, &fleet(Status::Running));
    let _ = app.model.select_name(&app.fleet, "web", 0);
    app.show();
    assert!(framed(&mut app).contains("to web"));
    assert!(!framed(&mut app).contains("(not home)"));
    let at = writing(&mut app);
    assert_eq!(app.compose(Key::Text(b"w".to_vec()), at), Some(()));
    drop(crate::store::open(&web).console_writer().expect("web free"));
    drop(crate::store::open(&api).console_writer().expect("api free"));
    let keys: Vec<_> = app.inputs.keys().collect();
    assert_eq!(keys, [&("web".to_owned(), ID.to_owned())]);
    let mut read = read_of(&app, &[("api", ID)]);
    (read.scanned, read.fleet) = (false, one_row(Some("api")));
    app.answer(Answer::Fleet(read));
    let typed = vec![(Key::Text(b"q".to_vec()), at)];
    assert_eq!(take_keys(&mut app, typed), Some(true), "never Quit");
    assert!(app.composing(), "an incomplete read keeps the writing");
    refold(&mut app, &reader, &root, &fleet(Status::Unknown));
    assert!(app.composing(), "an Unknown read revokes nothing");
    refold(&mut app, &reader, &root, &fleet(Status::Stopped));
    assert_eq!(app.held.as_deref(), Some("web is stopped"));
    let q = vec![(Key::Text(b"q".to_vec()), at)];
    assert_eq!(take_keys(&mut app, q), Some(false), "q stays swallowed");
    drop(crate::store::open(&web).console_writer().expect("released"));
    assert_eq!(drafted(&app), "wq", "its draft kept");
    let _ = take_keys(&mut app, vec![(Key::Escape, at), (Key::Enter, at)]);
    assert_eq!(app.held, None, "read-only Enter browses (D3)");
}

/// R-B4 ext (SELECTION-MOVED): a complete read that no longer lists the
/// writing target, or proves it gone, ends the writing HELD, never browse.
#[test]
fn a_complete_read_without_the_writing_target_ends_it_held() {
    let root = Root::new("unlisted");
    let (mut app, reader) = housed(&root);
    session(&root, "web", "");
    let both = [("api", Status::Running), ("web", Status::Running)];
    refold(&mut app, &reader, &root, &both);
    let _ = app.model.select_name(&app.fleet, "web", 0);
    writing(&mut app);
    let mut read = read_of(&app, &[("api", ID), ("web", ID)]);
    read.fleet.rows.retain(|row| row.name == "api");
    app.answer(Answer::Fleet(read));
    assert_eq!(app.held.as_deref(), Some("web is no longer listed"));
    refold(&mut app, &reader, &root, &both);
    let _ = app.model.select_name(&app.fleet, "web", 0);
    writing(&mut app);
    refold(&mut app, &reader, &root, &both[..1]);
    assert_eq!(app.held.as_deref(), Some("the session is gone"));
}

/// R-B4 ext (G2): a refused entry, like a replaced writer, is HELD with each read's reason.
#[test]
fn a_refused_entry_is_held_with_the_reason_each_read_gives() {
    let root = Root::new("held-read");
    let (mut app, reader) = housed(&root);
    // S2: another window's lease refuses no entry; a pair the meta does
    // not name refuses it instead.
    meta(&root, ID, "bad.name");
    app.write(true);
    assert_eq!(
        app.held.as_deref(),
        Some("the worker.0 seat is not an agent name")
    );
    refold(&mut app, &reader, &root, &[("api", Status::Stopped)]);
    assert_eq!(app.held.as_deref(), Some("api is stopped"));
    meta(&root, &ID.replace("1234", "bbbb"), "colead");
    refold(&mut app, &reader, &root, &[("api", Status::Running)]);
    assert_eq!(app.held.as_deref(), Some("the session was replaced"));
    assert!(
        app.entered.is_none() && app.writer.is_none(),
        "never acquires"
    );
    writing(&mut app);
    meta(&root, &ID.replace("1234", "cccc"), "colead");
    refold(&mut app, &reader, &root, &[("api", Status::Running)]);
    assert_eq!(app.held.as_deref(), Some("the session was replaced"));
}

/// #27/#28: `/close` answers with the chat admission's own answer: a
/// proven home with no tmux reaches the journal; a changed pair is
/// refused before it and ends the writing; no home session, no home.
#[test]
fn close_reports_the_chats_own_admission_answer() {
    let root = Root::new("close");
    let (mut app, _reader) = housed(&root);
    app.fleet = one_row(Some("api"));
    app.model = Model::new(&app.fleet);
    app.dirs
        .insert("api".to_owned(), root.0.join("sessions").join("api"));
    let typed = writing(&mut app);
    let close = |app: &mut App| {
        for key in [Key::Text(b"/close".to_vec()), Key::Enter] {
            assert_eq!(app.compose(key, typed), Some(()));
        }
        app.notices
            .last()
            .map_or_else(String::new, |said| said.1.body.clone())
    };
    assert_eq!(close(&mut app), "refused: no open ask to close");
    meta(&root, ID, "peer");
    let changed = "the lead pair changed since this chat opened - restart this chat";
    assert_eq!(close(&mut app), format!("refused: {changed}"));
    assert!(!app.composing(), "an unproven home ends the writing");
    assert_eq!(app.close(None), "refused: not writing");
}

/// A reseat changes a seat's profile and nothing the lead pair routes by:
/// the writing it ended keeps its admission and the next entry is not
/// refused, while a rename still is.
#[test]
fn a_reseat_keeps_the_writing_and_the_next_entry_but_a_rename_does_not() {
    let root = Root::new("reseat");
    let (mut app, reader) = housed(&root);
    let dir = root.0.join("sessions").join("api");
    let profiled = |main: &str, peer: &str| {
        let rows = format!("layout=lead-pair\nseat.main=lead\nprofile.main={main}\n");
        let rows = format!("{rows}seat.worker.0={peer}\nprofile.worker.0=sol61x\n");
        std::fs::write(
            dir.join("meta"),
            format!("schema=2\nsession_id={ID}\n{rows}"),
        )
        .expect("meta");
    };
    profiled("fable5", "colead");
    refold(&mut app, &reader, &root, &[("api", Status::Running)]);
    let typed = writing(&mut app);
    profiled("opus55x", "colead");
    let close = |app: &mut App| {
        for key in [Key::Text(b"/close".to_vec()), Key::Enter] {
            assert_eq!(app.compose(key, typed), Some(()));
        }
        app.notices
            .last()
            .map_or_else(String::new, |said| said.1.body.clone())
    };
    assert_eq!(close(&mut app), "refused: no open ask to close");
    assert!(app.composing(), "a reseat does not end the writing");
    refold(&mut app, &reader, &root, &[("api", Status::Running)]);
    app.write(false);
    app.write(true);
    assert!(app.composing() && app.held.is_none(), "{:?}", app.held);
    profiled("opus55x", "other");
    refold(&mut app, &reader, &root, &[("api", Status::Running)]);
    app.write(false);
    app.write(true);
    let changed = "the lead pair changed since this chat opened - restart this chat";
    assert_eq!(app.held.as_deref(), Some(changed), "a rename still refuses");
}

/// #36: seat facts come from the session's own picker row.
#[test]
fn seat_facts_come_from_the_sessions_own_picker_row() {
    let now = Timestamp::from_epoch(1_880);
    let picker = |name: &str, id: &str| tmux::PickerSession {
        name: name.to_owned(),
        id: id.to_owned(),
        rank: 0,
        glyph: String::new(),
        main_pane: String::new(),
        branch: String::new(),
        agents: "v1;1880;60;lead:fable5:working:".to_owned(),
        activity: None,
        created_at: None,
        goal: String::new(),
    };
    let rows = [picker("web", "$7"), picker("api", "$3")];
    let api = entry("api", Status::Running, None);
    assert!(
        matches!(facts_of(&api, Some(&rows), now), fleet::Facts::Seats { id, .. } if id == "$3")
    );
    assert!(matches!(
        facts_of(&api, None, now),
        fleet::Facts::NotPublished
    ));
    assert!(matches!(
        facts_of(&api, Some(&rows[..1]), now),
        fleet::Facts::OtherServer
    ));
}

/// #40: a dash is never a session name (grammar), so it is a usage error.
#[test]
fn a_dash_argument_is_a_usage_error_not_a_session() {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(&["-x".to_owned()], &mut out, &mut err).expect("writes");
    assert_eq!(code, crate::entry::EXIT_USAGE);
    assert_eq!(String::from_utf8_lossy(&err), USAGE);
}

/// #50/#56/#57, R2: browsing, only a second `q` within the window quits;
/// one arms, another key or a late `q` does not complete it.
#[test]
fn only_a_second_q_in_the_window_quits_while_browsing() {
    let mut app = app(None);
    app.fleet = one_row(None);
    app.model = Model::new(&app.fleet);
    let (t0, q) = (Instant::now(), Key::Text(b"q".to_vec()));
    assert!(browse(&mut app, &Key::Text(b"x".to_vec()), t0).is_some());
    assert!(browse(&mut app, &q, t0).is_some(), "one q only arms");
    assert_eq!(app.note().as_deref(), Some("q again to quit"));
    assert!(browse(&mut app, &Key::Text(b"x".to_vec()), t0).is_some());
    assert!(app.armed.is_none(), "another key disarms");
    assert!(browse(&mut app, &q, t0).is_some());
    let late = t0 + super::QUIT_WINDOW + Duration::from_millis(1);
    assert!(browse(&mut app, &q, late).is_some(), "a late q re-arms");
    assert!(app.lapse(late + super::QUIT_WINDOW * 2), "the arm expires");
    assert!(app.armed.is_none() && app.note().is_none());
    assert!(browse(&mut app, &q, late).is_some());
    assert!(browse(&mut app, &q, late).is_none(), "the second q quits");
}

/// R1/B1: a paste while browsing starts writing with it as the draft and
/// every later fragment of that read keeps its text, while text typed in
/// that read stays dropped; a selection that cannot be written says why.
#[test]
fn a_browse_paste_writes_whole_and_a_read_only_one_says_why() {
    let root = Root::new("paste");
    let (mut app, _reader) = housed(&root);
    let t0 = Instant::now();
    let paste = |bytes: &[u8]| (Key::Pasted(bytes.to_vec()), t0);
    assert_eq!(take_keys(&mut app, vec![paste(b"one ")]), Some(true));
    assert!(app.composing() && app.admitted == Some(t0));
    let typed = (Key::Text(b"X".to_vec()), t0);
    let early = t0.checked_sub(Duration::from_millis(1)).expect("booted");
    let other = (Key::Pasted(b"Y".to_vec()), early);
    let batch = vec![paste(b"two"), typed, other];
    assert_eq!(take_keys(&mut app, batch), Some(true));
    assert_eq!(
        drafted(&app),
        "one two",
        "typed text and another read's paste are dropped, the tail kept"
    );
    app.write(false);
    assert!(
        app.admitted.is_none(),
        "a way out of writing forgets the paste"
    );
    let mut cold = self::app(Some("api"));
    assert_eq!(take_keys(&mut cold, vec![paste(b"x")]), Some(true));
    let said = cold.note().expect("a refused paste says so");
    assert!(said.starts_with("paste not taken: read-only"), "{said}");
}

/// P1/C2: `^C` over text clears it whole and keeps it for `^Y`; only an
/// empty composer quits, so no `^C` over text ever ends the app.
#[test]
fn interrupt_over_text_clears_it_and_only_an_empty_composer_quits() {
    let root = Root::new("c2");
    let (mut app, _reader) = housed(&root);
    let at = writing(&mut app);
    let draft = |app: &App| app.shown_input().map(Input::draft).unwrap_or_default();
    assert_eq!(app.compose(Key::Interrupt, at), None, "empty quits at once");
    assert_eq!(app.compose(Key::Text(b"keep\nme".to_vec()), at), Some(()));
    assert_eq!(take_keys(&mut app, vec![(Key::Interrupt, at)]), Some(true));
    assert_eq!(draft(&app), "", "every line is gone");
    let line = "draft cleared · ^Y brings it back";
    assert_eq!(app.note().as_deref(), Some(line));
    assert!(app.composing() && app.armed.is_none(), "still writing");
    assert_eq!(take_keys(&mut app, vec![(Key::Yank, at)]), Some(true));
    assert_eq!(draft(&app), "keep me", "^Y brings it back");
    assert_eq!(take_keys(&mut app, vec![(Key::Yank, at)]), Some(true));
    assert_eq!(draft(&app), "keep me", "and only once");
    assert_eq!(take_keys(&mut app, vec![(Key::ClearLine, at)]), Some(true));
    assert_eq!(draft(&app), "", "^U clears the same way");
    assert_eq!(app.note().as_deref(), Some(line));
    assert_eq!(take_keys(&mut app, vec![(Key::Yank, at)]), Some(true));
    assert_eq!(take_keys(&mut app, vec![(Key::Interrupt, at)]), Some(true));
    assert_eq!(draft(&app), "", "cleared again");
}

/// A `^C` meant for the text must not end the app: just after a clear an
/// empty-composer `^C` only arms, a further one inside the guard quits, and
/// so does one after the guard ran out or with no clear before it.
#[test]
fn interrupt_just_after_a_clear_arms_and_only_a_further_one_quits() {
    let root = Root::new("c2-guard");
    let (mut app, _reader) = housed(&root);
    let at = writing(&mut app);
    let guard = super::QUIT_GUARD;
    let (cleared, armed) = ("draft cleared · ^Y brings it back", "^C again to quit");
    assert_eq!(app.compose(Key::Text(b"keep".to_vec()), at), Some(()));
    assert_eq!(take_keys(&mut app, vec![(Key::Interrupt, at)]), Some(true));
    assert_eq!(app.note().as_deref(), Some(cleared));
    let soon = at + Duration::from_millis(900);
    let press = vec![(Key::Interrupt, soon)];
    assert_eq!(take_keys(&mut app, press.clone()), Some(true));
    let said = app.note().expect("armed");
    assert!(said.starts_with(armed) && said.contains("^Y"), "{said}");
    assert!(app.composing(), "alive, still writing");
    assert_eq!(take_keys(&mut app, press), None, "a further one quits");
    // ^U clears the same way and guards the same way.
    let root = Root::new("c2-guard-1");
    let (mut app, _reader) = housed(&root);
    let at = writing(&mut app);
    assert_eq!(app.compose(Key::Text(b"keep".to_vec()), at), Some(()));
    assert_eq!(take_keys(&mut app, vec![(Key::ClearLine, at)]), Some(true));
    assert_eq!(take_keys(&mut app, vec![(Key::Interrupt, at)]), Some(true));
    assert!(app.note().is_some_and(|n| n.starts_with(armed)));
    // Another key disarms; the guard still runs, so the next one arms again.
    assert_eq!(take_keys(&mut app, vec![(Key::Left, at)]), Some(true));
    assert!(app.armed.is_none());
    assert_eq!(take_keys(&mut app, vec![(Key::Interrupt, at)]), Some(true));
    assert!(app.note().is_some_and(|n| n.starts_with(armed)));
    // Past the guard an empty ^C quits at once, armed or not.
    let late = at + guard + Duration::from_millis(1);
    assert_eq!(take_keys(&mut app, vec![(Key::Interrupt, late)]), None);
    let root = Root::new("c2-guard-2");
    let (mut app, _reader) = housed(&root);
    let at = writing(&mut app);
    assert_eq!(app.compose(Key::Text(b"keep".to_vec()), at), Some(()));
    assert_eq!(take_keys(&mut app, vec![(Key::Interrupt, at)]), Some(true));
    let late = at + guard + Duration::from_millis(1);
    assert_eq!(take_keys(&mut app, vec![(Key::Interrupt, late)]), None);
    // No clear before it: an empty composer quits as it always did.
    let root = Root::new("c2-guard-3");
    let (mut app, _reader) = housed(&root);
    let at = writing(&mut app);
    assert_eq!(take_keys(&mut app, vec![(Key::Interrupt, at)]), None);
}

/// `^W`, `Opt+Backspace` and `^K` delete in the composer the app is writing
/// and fill the `^Y` slot; one that leaves it empty guards the quit like `^U`,
/// one that leaves text does not.
#[test]
fn a_word_or_line_kill_feeds_ctrl_y_and_guards_the_quit_only_when_it_empties_the_draft() {
    let root = Root::new("c2-kill");
    let (mut app, _reader) = housed(&root);
    let at = writing(&mut app);
    let draft = |app: &App| app.shown_input().map(Input::draft).unwrap_or_default();
    let press = |app: &mut App, at: Instant, bytes: &[u8]| {
        take_keys(app, crate::console::input::Keys::app().feed(bytes, at))
    };
    let armed = "^C again to quit";
    assert_eq!(app.compose(Key::Text(b"keep me".to_vec()), at), Some(()));
    assert_eq!(press(&mut app, at, b"\x17"), Some(true));
    assert_eq!(draft(&app), "keep ", "^W took the last word");
    assert!(app.cleared_at.is_none(), "text is left: no guard");
    assert_eq!(press(&mut app, at, b"\x1b\x7f"), Some(true));
    assert_eq!(draft(&app), "", "Opt+Backspace took the other");
    assert_eq!(
        app.note().as_deref(),
        Some("draft cleared · ^Y brings it back"),
        "the draft is gone, so the clear's note"
    );
    assert_eq!(press(&mut app, at, b"\x03"), Some(true));
    assert!(
        app.note().is_some_and(|n| n.starts_with(armed)),
        "armed, not quit"
    );
    assert_eq!(press(&mut app, at, b"\x19"), Some(true));
    assert_eq!(draft(&app), "keep ", "^Y brings back the newest kill");
    let root = Root::new("c2-kill-1");
    let (mut app, _reader) = housed(&root);
    let at = writing(&mut app);
    assert_eq!(app.compose(Key::Text(b"keep me".to_vec()), at), Some(()));
    assert_eq!(press(&mut app, at, b"\x01\x0b"), Some(true));
    assert_eq!(draft(&app), "", "^K from the start takes the line");
    assert_eq!(press(&mut app, at, b"\x03"), Some(true), "guarded");
    assert_eq!(press(&mut app, at, b"\x19"), Some(true));
    assert_eq!(draft(&app), "keep me");
}

/// `^C` clears text wherever the composer shows it: the draft Esc kept in
/// browse, and one under an open overlay. Neither quits.
#[test]
fn interrupt_clears_a_kept_draft_and_one_under_the_settings_overlay() {
    let root = Root::new("c2-kept");
    let (mut app, _reader) = housed(&root);
    let at = writing(&mut app);
    assert_eq!(app.compose(Key::Text(b"keep".to_vec()), at), Some(()));
    assert_eq!(take_keys(&mut app, vec![(Key::Escape, at)]), Some(true));
    assert!(!app.composing() && app.has_text(), "Esc kept the draft");
    assert_eq!(take_keys(&mut app, vec![(Key::Interrupt, at)]), Some(true));
    assert!(!app.has_text(), "browse: cleared, not quit");
    assert!(app.note().is_some_and(|n| n.contains("Enter, then ^Y")));
    let again = vec![(Key::Interrupt, at)];
    assert_eq!(take_keys(&mut app, again.clone()), Some(true), "guarded");
    assert_eq!(take_keys(&mut app, again), None, "a further one quits");
    let (mut app, _reader) = housed(&root);
    let at = writing(&mut app);
    assert_eq!(app.compose(Key::Text(b"keep".to_vec()), at), Some(()));
    app.open_settings();
    assert_eq!(take_keys(&mut app, vec![(Key::Interrupt, at)]), Some(true));
    assert!(
        app.model.settings_open() && !app.has_text(),
        "cleared under it"
    );
    let again = vec![(Key::Interrupt, at)];
    assert_eq!(take_keys(&mut app, again.clone()), Some(true), "guarded");
    assert_eq!(take_keys(&mut app, again), None, "a further one quits");
}

/// P2/O1: the first `o` arms on the frame it saw and moves nothing; the
/// second within the window opens THAT seat, though a later frame moved
/// the highlight or drew no seat at all; a late second only arms again.
#[test]
fn an_armed_open_keeps_the_seat_it_saw() {
    let root = Root::new("armed-open");
    let (mut app, _reader) = housed(&root);
    let frame = |focused: &str| super::Drawn {
        rows: vec![
            ("lead".to_owned(), Some(seat_ref("main", "lead"))),
            ("colead".to_owned(), Some(seat_ref("worker.0", "colead"))),
        ],
        ..drawn_rows(&[], focused)
    };
    let (t0, said) = (Instant::now(), |app: &App| {
        app.notices.last().map(|n| n.1.body.clone())
    });
    app.drawn = Some(frame("lead"));
    app.open_key(t0);
    assert_eq!(app.note().as_deref(), Some("o again to open lead"));
    assert_eq!(said(&app), None, "the first press opens nothing");
    app.drawn = Some(frame("colead"));
    app.open_key(t0 + Duration::from_millis(500));
    let opened = "refused: /open lead: ae app is not running inside tmux; nothing selected";
    assert_eq!(said(&app).as_deref(), Some(opened), "the captured seat");
    app.open_key(t0 + Duration::from_secs(9));
    assert_eq!(app.note().as_deref(), Some("o again to open colead"));
    // I1: a refresh that drew no seat still completes the stored arm.
    app.notices.clear();
    app.drawn = None;
    app.open_key(t0 + Duration::from_millis(9500));
    let colead = "refused: /open colead: ae app is not running inside tmux; nothing selected";
    assert_eq!(said(&app).as_deref(), Some(colead), "the stored seat");
    app.drawn = Some(frame("colead"));
    app.open_key(t0 + Duration::from_secs(9));
    let moved = Fleet {
        rows: vec![row("web", 1, true)],
        home: Some("web".to_owned()),
    };
    app.model = Model::new(&moved);
    app.open_key(t0 + Duration::from_secs(10));
    assert!(said(&app).is_some_and(|line| line.contains("the view changed")));
}

/// I5: a `^C` held behind other keys quits at once, unless it would lose a
/// draft only this process holds - then it waits, in order, with them.
#[test]
fn a_held_interrupt_quits_at_once_unless_it_would_lose_a_draft() {
    let root = Root::new("held-quit");
    let (mut app, _reader) = housed(&root);
    let at = writing(&mut app);
    let behind = |app: &mut App| {
        app.deferred = vec![(Key::Text(b"x".to_vec()), at)];
        take_keys(app, vec![(Key::Interrupt, at)])
    };
    assert_eq!(behind(&mut app), None, "nothing to lose: it quits");
    assert_eq!(app.compose(Key::Text(b"keep".to_vec()), at), Some(()));
    assert_eq!(behind(&mut app), Some(true), "a draft is not lost");
    assert_eq!(app.deferred.len(), 2, "the ^C waits behind the key");
}

/// I5: a held `^C` keeps the guard of its own press: one made inside
/// [`QUIT_GUARD`] of a clear waits however long the clock runs meanwhile and
/// arms when it replays; one made past it quits at once.
#[test]
fn a_held_interrupt_keeps_the_guard_of_its_own_press() {
    let root = Root::new("held-guard");
    let (mut app, _reader) = housed(&root);
    let at = writing(&mut app);
    let cleared = (at.checked_sub(Duration::from_secs(3))).expect("a clock 3 s past boot");
    let behind = |app: &mut App, pressed: Instant| {
        app.cleared_at = Some(cleared);
        app.deferred = vec![(Key::Left, cleared)];
        take_keys(app, vec![(Key::Interrupt, pressed)])
    };
    let inside = cleared + Duration::from_secs(1);
    let kept = behind(&mut app, inside);
    assert_eq!(kept, Some(true), "pressed inside the guard it waits");
    assert_eq!(app.deferred.len(), 2, "the ^C waits behind the key");
    let held = std::mem::take(&mut app.deferred);
    assert_eq!(take_keys(&mut app, held), Some(true));
    let armed = app.note();
    assert!(
        armed.is_some_and(|said| said.starts_with("^C again to quit")),
        "replayed, it arms instead of quitting"
    );
    let past = cleared + super::QUIT_GUARD + Duration::from_millis(1);
    assert_eq!(behind(&mut app, past), None, "past the guard it quits");
}

/// The guard after a clear is [`QUIT_GUARD`] long, inclusive, counted from
/// the clear to the press's own read; with no clear there is no guard, and
/// a press read before the clear saturates to the instant of the clear.
#[test]
fn a_press_is_guarded_for_the_guard_window_after_a_clear() {
    let before = Instant::now();
    let cleared = before + Duration::from_secs(10);
    let guard = super::QUIT_GUARD;
    let ms = Duration::from_millis(1);
    let guarded = |at| super::guarded_at(Some(cleared), at);
    assert!(guarded(cleared), "the clear's own instant");
    assert!(guarded(cleared + guard), "the last instant of the window");
    assert!(!guarded(cleared + guard + ms), "one tick past it");
    assert!(guarded(before), "read before the clear saturates");
    assert!(!super::guarded_at(None, cleared), "no clear, no guard");
}

/// Held keys quit at once only for a `^C` the clear does not guard, with no
/// ask queued and no draft text to lose; any other key never quits them.
#[test]
fn held_keys_quit_only_for_an_unguarded_interrupt_with_nothing_to_lose() {
    let t0 = Instant::now();
    let cleared = t0 + Duration::from_secs(10);
    let late = cleared + super::QUIT_GUARD + Duration::from_millis(1);
    let inside = cleared + Duration::from_secs(1);
    let ints = |at: Instant| (Key::Interrupt, at);
    let text = (Key::Text(b"j".to_vec()), t0);
    let quits = super::held_quit;
    assert!(quits(&[ints(t0)], true, false, None), "no clear: quits");
    assert!(!quits(&[], true, false, None), "nothing held");
    assert!(!quits(std::slice::from_ref(&text), true, false, None));
    assert!(!quits(&[ints(t0)], false, false, None), "an ask is queued");
    assert!(!quits(&[ints(t0)], true, true, None), "a draft has text");
    assert!(
        !quits(&[ints(inside)], true, false, Some(cleared)),
        "guarded"
    );
    assert!(
        quits(&[ints(late)], true, false, Some(cleared)),
        "unguarded"
    );
    let both = [text, ints(inside), ints(late)];
    assert!(
        quits(&both, true, false, Some(cleared)),
        "any unguarded one"
    );
    assert!(
        !quits(&both[..2], true, false, Some(cleared)),
        "only guarded"
    );
}

/// Which mode takes a key is the first of settings, seat view, reader,
/// composer, held entry that is open, else browsing (docs/app.md: an open
/// Settings or seat view owns its keys even over a kept draft, a held
/// composer takes every key). All 32 combinations, the winner read as the
/// highest set bit of (settings, peek, reader, composing, held).
#[test]
fn the_first_open_mode_takes_the_key() {
    use super::Mode::{Browse, Composer, Held, Peek, Reader, Settings};
    let by_height = [Browse, Held, Composer, Reader, Peek, Settings];
    for bits in 0_u32..32 {
        let on = |bit: u32| bits & (1 << bit) != 0;
        let got = super::mode([on(4), on(3), on(2)], on(1), on(0));
        let top = (u32::BITS - bits.leading_zeros()) as usize;
        assert_eq!(got, by_height[top], "open modes {bits:05b}");
    }
    assert_eq!(super::mode([true, false, false], true, false), Settings);
    assert_eq!(super::mode([false, true, false], true, false), Peek);
    assert_eq!(super::mode([false; 3], false, true), Held);
}

/// With two overlays open, the earlier of settings, seat view, reader owns
/// the key at the call site too, not just in [`mode`]: Esc is handled as
/// that one's close, which tells the reader thread of exactly that one.
#[test]
fn esc_is_the_close_of_the_first_open_overlay() {
    let at = Instant::now();
    let held = |open: fn(&mut Model), asks: &mut Vec<loader::Request>| {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = App::new(None, None, None, Some(tx));
        app.fleeted = true;
        open(&mut app.model);
        assert_eq!(take_keys(&mut app, vec![(Key::Escape, at)]), Some(true));
        asks.extend(rx.try_iter());
    };
    let mut told = Vec::new();
    held(
        |model| {
            model.open_peek(1);
            model.open_settings(1);
        },
        &mut told,
    );
    assert!(
        told.iter()
            .any(|ask| matches!(ask, loader::Request::Settings { open: false, .. })),
        "settings took the Esc and said it closed"
    );
    assert!(
        !told
            .iter()
            .any(|ask| matches!(ask, loader::Request::Peek(_)))
    );
    let mut told = Vec::new();
    held(
        |model| {
            model.open_reader(1);
            model.open_peek(1);
        },
        &mut told,
    );
    assert!(
        told.iter()
            .any(|ask| matches!(ask, loader::Request::Peek(None))),
        "the seat view took the Esc and said it closed"
    );
}

/// An armed press survives `^C` and, while browsing, a letter; any other
/// key ends it (docs/app.md §Keys: "disarmed by any other key").
#[test]
fn only_interrupt_and_a_browsing_letter_keep_an_armed_press() {
    let letter = Key::Text(b"j".to_vec());
    for browsing in [true, false] {
        assert!(!super::disarms(&Key::Interrupt, browsing), "^C {browsing}");
        for other in [Key::Down, Key::Enter, Key::Escape, Key::Tab] {
            assert!(super::disarms(&other, browsing), "{other:?} {browsing}");
        }
    }
    assert!(!super::disarms(&letter, true), "a browsing letter");
    assert!(super::disarms(&letter, false), "a letter in a draft");
}

/// O1: a press after the window passed arms the seat the CURRENT frame
/// highlights, not the stale one; a frame of the selection with no seat to
/// open answers at once and arms nothing.
#[test]
fn an_expired_open_rearms_the_current_seat_and_a_seatless_frame_refuses() {
    let root = Root::new("open-expired");
    let (mut app, _reader) = housed(&root);
    let frame = |focused: &str| super::Drawn {
        rows: vec![
            ("lead".to_owned(), Some(seat_ref("main", "lead"))),
            ("colead".to_owned(), Some(seat_ref("worker.0", "colead"))),
        ],
        ..drawn_rows(&[], focused)
    };
    let t0 = Instant::now();
    app.drawn = Some(frame("lead"));
    app.open_key(t0);
    app.drawn = Some(frame("colead"));
    app.open_key(t0 + super::QUIT_WINDOW + Duration::from_millis(1));
    assert_eq!(app.note().as_deref(), Some("o again to open colead"));
    assert!(app.notices.is_empty(), "the stale arm opened nothing");
    app.armed = None;
    app.drawn = Some(super::Drawn {
        focused: None,
        ..frame("lead")
    });
    app.open_key(t0 + Duration::from_secs(9));
    assert!(app.armed.is_none(), "no seat, nothing armed");
    let said = (app.notices.last()).map_or_else(String::new, |n| n.1.body.clone());
    assert!(
        said.starts_with("refused: /open api"),
        "answered at once: {said}"
    );
}

/// P1: the settled word of an ask shows on the hint row of the session it
/// answers and of no other.
#[test]
fn a_settled_word_shows_on_its_own_sessions_hint_row_only() {
    let root = Root::new("settled-word");
    let (mut app, _reader) = housed(&root);
    let settling = |about: &str| super::Settling {
        at: Instant::now(),
        id: "r".to_owned(),
        word: "delivered".to_owned(),
        about: about.to_owned(),
    };
    app.settling = Some(settling("api"));
    assert_eq!(app.note().as_deref(), Some("delivered"));
    app.settling = Some(settling("web"));
    assert_eq!(app.note(), None, "another session's word is not shown");
}

/// P1: a line lapses once its window has passed, not at the window itself,
/// and `lapse` says so whenever any of the three went away.
#[test]
fn a_line_lapses_after_its_window_and_any_lapse_is_said() {
    let root = Root::new("lapse");
    let (mut app, _reader) = housed(&root);
    let t0 = Instant::now();
    let past = |window: Duration| t0 + window + Duration::from_millis(1);
    let arm = |on: bool| {
        on.then_some(super::Armed {
            at: t0,
            what: super::Arm::Quit,
        })
    };
    let flash = |on: bool| on.then(|| ("paste not taken: x".to_owned(), t0));
    let word = |on: bool| {
        on.then(|| super::Settling {
            at: t0,
            id: "r".to_owned(),
            word: "delivered".to_owned(),
            about: "api".to_owned(),
        })
    };
    app.armed = arm(true);
    assert!(
        !app.lapse(t0 + super::QUIT_WINDOW),
        "at the window it still waits"
    );
    assert!(app.armed.is_some());
    for (on, at) in [
        ((true, false, false), past(super::QUIT_WINDOW)),
        ((true, true, false), past(super::FLASH_WINDOW)),
        ((true, false, true), past(super::FLASH_WINDOW)),
        ((false, true, true), past(super::FLASH_WINDOW)),
        ((true, true, true), past(super::FLASH_WINDOW)),
        ((false, true, false), past(super::FLASH_WINDOW)),
        ((false, false, true), past(super::FLASH_WINDOW)),
    ] {
        (app.armed, app.flash, app.settling) = (arm(on.0), flash(on.1), word(on.2));
        assert!(app.lapse(at), "{on:?} lapsed");
        let left = (app.flash.is_some(), app.settling.is_some());
        assert_eq!(
            left,
            (false, false),
            "{on:?}: flash and word are over by {at:?}"
        );
    }
    app.flash = flash(true);
    assert!(
        !app.lapse(t0 + super::FLASH_WINDOW),
        "at the window it still waits"
    );
}

/// B1: a paste while the app is HELD is refused with a hint, every
/// fragment of it again, and starts no draft.
#[test]
fn a_paste_while_held_is_refused_on_the_hint_row_fragment_by_fragment() {
    let root = Root::new("held-paste");
    let (mut app, _reader) = housed(&root);
    app.held = Some("an ae app is writing to api (holder unknown)".to_owned());
    let at = Instant::now();
    for fragment in [b"one".as_slice(), b"two"] {
        app.flash = None;
        let pasted = vec![(Key::Pasted(fragment.to_vec()), at)];
        assert_eq!(take_keys(&mut app, pasted), Some(true));
        let said = app.note().expect("a held paste says why");
        assert!(
            said.starts_with("paste not taken: ") && said.contains("Esc"),
            "{said}"
        );
    }
    assert!(
        !app.composing() && drafted(&app).is_empty(),
        "no draft started"
    );
}

/// A read-only selection's refused paste carries the real reason the
/// session cannot be written, in full.
#[test]
fn a_refused_browse_paste_names_the_sessions_real_reason() {
    let mut app = self::app(Some("api"));
    app.read_line = "api is stopped; ae api resumes it".to_owned();
    let pasted = vec![(Key::Pasted(b"x".to_vec()), Instant::now())];
    assert_eq!(take_keys(&mut app, pasted), Some(true));
    let line = "paste not taken: read-only · api is stopped; ae api resumes it";
    assert_eq!(app.note().as_deref(), Some(line));
}

/// `app` drawn into a `width` x 45 buffer, as text.
fn framed_at(app: &mut App, width: u16) -> String {
    let mut buf = Buffer::empty(Rect::new(0, 0, width, 45));
    app.frame(&mut buf);
    text(&buf)
}

/// P1: a pending line takes the hint row, the pane's last row.
#[test]
fn a_pending_line_is_drawn_on_the_hint_row() {
    let root = Root::new("hint-row");
    let (mut app, _reader) = housed(&root);
    app.refuse_paste("read-only");
    let shown = framed_at(&mut app, 160);
    let rows: Vec<&str> = shown.lines().collect();
    assert!(rows[44].contains("paste not taken: read-only"), "{shown}");
    assert!(rows[43].trim().is_empty() || !rows[43].contains("paste not taken"));
}

/// The settings overlay puts three cells between its tab titles when all of
/// them fit that way, else one.
#[test]
fn the_settings_tab_titles_keep_three_cells_only_while_all_fit_so() {
    let root = Root::new("tab-gaps");
    let (mut app, _reader) = housed(&root);
    app.open_settings();
    let row = |app: &mut App, width: u16| {
        let shown = framed_at(app, width);
        shown
            .lines()
            .nth(1)
            .expect("the tab row")
            .trim_end()
            .to_owned()
    };
    // Seven titles: 31 compact cells or 49 full cells, plus LEFT=2.
    // Six gaps need 6 cells packed or 18 cells wide: boundaries 51/57/69.
    let compact = "  Quota Conf About Instr Keys Cli Stats";
    let compact_wide = "  Quota   Conf   About   Instr   Keys   Cli   Stats";
    let full = "  Quota Config About Instructions Keys Clients Statistics";
    let full_wide = "  Quota   Config   About   Instructions   Keys   Clients   Statistics";
    for width in [40, 44, 45, 46, 50] {
        assert_eq!(row(&mut app, width), compact, "width {width}");
    }
    for width in [51, 56] {
        assert_eq!(row(&mut app, width), compact_wide, "width {width}");
    }
    for width in [57, 68] {
        assert_eq!(row(&mut app, width), full, "width {width}");
    }
    for width in [69, 70] {
        assert_eq!(row(&mut app, width), full_wide, "width {width}");
    }
}

/// R4(f)/I5: Enter queues its ask behind the frame that names it; a key
/// behind it waits in order, an answer behind it waits in the channel,
/// and a `^C` behind it acts only after the ask ran: over the draft the
/// refusal gave back it clears, and only a second quits.
#[test]
fn an_entered_ask_is_delivered_before_anything_behind_it_acts() {
    let root = Root::new("queued");
    let (mut app, _reader) = housed(&root);
    let at = writing(&mut app);
    // The admission refuses the ask, so nothing is really delivered.
    meta(&root, &ID.replace("1234", "bbbb"), "colead");
    let (wake, wakes) = std::sync::mpsc::channel();
    let mut keys = crate::console::input::Keys::app();
    let late = super::Wake::Answer(Box::new(view("api", ID, 9, "late")));
    wake.send(super::Wake::Keys(at, b"hi\r\x03".to_vec()))
        .expect("sent");
    wake.send(late).expect("sent");
    assert_eq!(
        super::drain(&mut app, &mut keys, &wakes, &mut None),
        Some(true)
    );
    assert_eq!((app.queued.len(), app.deferred.len()), (1, 1), "^C waits");
    assert_eq!(app.note().as_deref(), Some("sending to lead…"));
    assert!(wakes.try_recv().is_ok(), "the answer was not pulled");
    assert!(app.deliver() && app.queued.is_empty());
    let notice = &app.notices.last().expect("said").1.body;
    assert_eq!(notice, "refused: the session was replaced or renamed");
    assert!(app.has_text(), "the refused line is back in the draft");
    let drained = super::drain(&mut app, &mut keys, &wakes, &mut None);
    assert_eq!(drained, Some(true), "the held ^C clears it, not quits");
    assert!(!app.has_text());
    for step in [Some(true), None] {
        wake.send(super::Wake::Keys(at, b"\x03".to_vec()))
            .expect("sent");
        let drained = super::drain(&mut app, &mut keys, &wakes, &mut None);
        assert_eq!(drained, step, "guarded once, then a further ^C quits");
    }
}

/// I5: an ask whose writer ended while it waited asks nothing: the line
/// goes back to its draft in memory and the human is told.
#[test]
fn a_queued_ask_whose_writer_ended_is_not_sent() {
    let root = Root::new("revoked-queue");
    let dir = root.0.join("sessions").join("api");
    let (mut app, _reader) = housed(&root);
    let at = writing(&mut app);
    for key in [Key::Text(b"keep me".to_vec()), Key::Enter] {
        assert_eq!(app.compose(key, at), Some(()));
    }
    app.write(false);
    assert!(app.deliver());
    assert_eq!(drafted(&app), "keep me");
    let notice = &app.notices.last().expect("said").1.body;
    assert!(
        notice.starts_with("not sent to lead: writing ended"),
        "{notice}"
    );
    let written = crate::store::open(&dir).events_source();
    assert!(matches!(written, crate::store::SourceRead::Absent));
}

/// I5/B1: a held `Enter` replays and queues its ask; the wake read before
/// it (a revocation, the input closing) waits for the delivery, then acts.
#[test]
fn a_prefetched_wake_waits_for_the_ask_a_held_enter_queued() {
    for closed in [false, true] {
        let root = Root::new("prefetch");
        let (mut app, _reader) = housed(&root);
        let at = writing(&mut app);
        meta(&root, &ID.replace("1234", "bbbb"), "colead");
        assert_eq!(app.compose(Key::Text(b"hi".to_vec()), at), Some(()));
        app.deferred = vec![(Key::Enter, at)];
        let entry = app.writer.as_ref().map(|w| w.entry).expect("writing");
        let reading = Reading::NotOwner("gone".to_owned());
        let owned = Answer::Owned { entry, reading, at };
        let mut first = Some(super::Wake::Answer(Box::new(owned)));
        if closed {
            first = Some(super::Wake::Closed);
        }
        let (_wake, wakes) = std::sync::mpsc::channel();
        let mut keys = crate::console::input::Keys::app();
        let drain = super::drain(&mut app, &mut keys, &wakes, &mut first);
        assert!(drain.is_some() && first.is_some(), "kept, and no quit");
        assert_eq!(app.queued.len(), 1, "the held Enter queued its ask");
        assert!(app.composing(), "the revocation waits for the delivery");
        assert!(app.deliver() && app.queued.is_empty());
        let drain = super::drain(&mut app, &mut keys, &wakes, &mut first);
        assert_eq!(drain.is_none(), closed, "then the wake acts");
    }
}

/// I5: behind a queued ask the rest of the read waits in order - an
/// `Esc`, a second `Enter`, a wheel notch - and none acts before it.
#[test]
fn keys_behind_an_entered_ask_wait_in_order_for_its_delivery() {
    let wheel = super::Mouse {
        kind: super::MouseKind::WheelDown,
        column: 60,
        row: 10,
    };
    for behind in [Key::Escape, Key::Enter, Key::Mouse(wheel)] {
        let root = Root::new("queued-order");
        let (mut app, _reader) = housed(&root);
        let at = writing(&mut app);
        meta(&root, &ID.replace("1234", "bbbb"), "colead");
        let hi = (Key::Text(b"hi".to_vec()), at);
        let read = vec![hi, (Key::Enter, at), (behind.clone(), at)];
        assert_eq!(take_keys(&mut app, read), Some(true));
        let (queued, held) = (app.queued.len(), app.deferred.len());
        assert_eq!((queued, held), (1, 1), "{behind:?} waits behind one ask");
        assert!(app.composing() && app.deliver());
    }
}

/// `api` writing with `web` drawn under it, then a fleet read that `read`
/// reorders, truncates or swaps (`ops` in `web`'s place): the app and the
/// press on `web`'s card, made before that read's first frame.
fn pressed_web(root: &Root, read: &str) -> (App, Instant, String) {
    let (mut app, reader) = housed(root);
    session(root, "web", "");
    let both = [("api", Status::Running), ("web", Status::Running)];
    refold(&mut app, &reader, root, &both);
    let at = writing(&mut app);
    meta(root, &ID.replace("1234", "bbbb"), "colead");
    let shown = framed(&mut app);
    let at_row = shown.lines().position(|line| line.contains("web"));
    let mut data = read_of(&app, &[("api", ID), ("web", ID)]);
    match read {
        "order" => data.fleet.rows.reverse(),
        "gone" => data.fleet.rows.truncate(1),
        _ => data.fleet.rows[1] = row("ops", 2, false),
    }
    app.answer(Answer::Fleet(data));
    (
        app,
        at,
        format!("\x1b[<0;3;{}M", at_row.expect("web row") + 1),
    )
}

/// B2: a click held behind a queued ask acts on the row it was made on, whatever the
/// sending frame drew of a fleet read; one made after that frame acts on what it drew.
#[test]
fn a_click_held_behind_a_queued_ask_acts_on_the_row_it_was_made_on() {
    for (read, held, fresh) in [
        ("swap", "api", "ops"),
        ("order", "web", "api"),
        ("gone", "api", "api"),
    ] {
        let root = Root::new(&format!("click-held-{read}"));
        let (mut app, at, click) = pressed_web(&root, read);
        let mut keys = crate::console::input::Keys::app();
        let mut keyed = vec![(Key::Text(b"hi".to_vec()), at), (Key::Enter, at)];
        keyed.extend(keys.feed(click.as_bytes(), Instant::now()));
        assert_eq!(take_keys(&mut app, keyed), Some(true));
        assert_eq!((app.queued.len(), app.deferred.len()), (1, 1));
        let _sending = framed(&mut app);
        assert!(app.deliver() && app.queued.is_empty());
        let (_wake, wakes) = std::sync::mpsc::channel();
        assert!(super::drain(&mut app, &mut keys, &wakes, &mut None).is_some());
        assert_eq!(app.model.selected(), Some(held), "{read}: held click");
        let _ = take_keys(&mut app, keys.feed(click.as_bytes(), Instant::now()));
        assert_eq!(app.model.selected(), Some(fresh), "{read}: later click");
    }
}

/// B2: a second ask queued behind the first repaints once more; the click
/// held behind both still acts on the row it was made on. A refusal ends the
/// writing, so the lease is taken again for the held Enter to queue its ask.
#[test]
fn a_click_held_behind_two_queued_asks_acts_on_the_row_it_was_made_on() {
    let root = Root::new("click-held-two");
    let (mut app, at, click) = pressed_web(&root, "swap");
    let mut keys = crate::console::input::Keys::app();
    let soon = Instant::now() + Duration::from_mins(1);
    let text = |bytes: &[u8], at| (Key::Text(bytes.to_vec()), at);
    let mut keyed = vec![
        text(b"a", at),
        (Key::Enter, at),
        text(b"b", soon),
        (Key::Enter, soon),
    ];
    keyed.extend(keys.feed(click.as_bytes(), Instant::now()));
    assert_eq!(take_keys(&mut app, keyed), Some(true));
    let (_wake, wakes) = std::sync::mpsc::channel();
    for queued in [1, 0] {
        let _sending = framed(&mut app);
        assert!(app.deliver() && app.queued.is_empty());
        if queued == 1 {
            (app.held, ()) = (None, meta(&root, ID, "colead"));
            writing(&mut app);
            meta(&root, &ID.replace("1234", "bbbb"), "colead");
        }
        assert!(super::drain(&mut app, &mut keys, &wakes, &mut None).is_some());
        assert_eq!(app.queued.len(), queued, "the second ask queues behind");
    }
    assert_eq!(app.model.selected(), Some("api"), "no retarget");
}

/// B2: a click the terminal splits over three reads is begun at the first,
/// and the frames that drew since stay kept for it until its last byte lands.
#[test]
fn a_click_split_across_reads_acts_on_the_row_it_was_begun_on() {
    let root = Root::new("click-split");
    let (mut app, _at, click) = pressed_web(&root, "swap");
    let (mut keys, (_wake, wakes)) = (
        crate::console::input::Keys::app(),
        std::sync::mpsc::channel(),
    );
    let click = click.as_bytes();
    let mut read = |app: &mut App, bytes: &[u8]| {
        let mut first = Some(loader::Wake::Keys(Instant::now(), bytes.to_vec()));
        assert!(super::drain(app, &mut keys, &wakes, &mut first).is_some());
    };
    read(&mut app, &[b"a\r", &click[..3]].concat());
    assert_eq!((app.queued.len(), app.deferred.len()), (1, 0));
    let _sending = framed(&mut app);
    assert!(app.deliver() && app.queued.is_empty());
    read(&mut app, &click[3..click.len() - 1]);
    let _outcome = framed(&mut app);
    read(&mut app, &click[click.len() - 1..]);
    assert_eq!(app.model.selected(), Some("api"), "no retarget");
    assert!(app.history.is_empty(), "nothing kept once it landed");
}

/// B2: a report that never ends keeps the eight newest frames and its click
/// is not taken, said on the hint row, yet it still ends a drag left open;
/// one the decoder discarded keeps none and ends nothing.
#[test]
fn the_frames_kept_for_an_unfinished_click_are_bounded() {
    let gone = Some("click not taken: its frame is gone");
    for (filler, kept, said) in [(20, 8, gone), (40, 0, None)] {
        let grab = Drag {
            edge: Edge::Sidebar,
            from: 44,
            at: 44,
        };
        let root = Root::new(&format!("click-bound-{filler}"));
        let (mut app, _at, click) = pressed_web(&root, "swap");
        let (mut keys, (_wake, wakes)) = (
            crate::console::input::Keys::app(),
            std::sync::mpsc::channel(),
        );
        let mut read = |app: &mut App, bytes: &[u8]| {
            let mut first = Some(loader::Wake::Keys(Instant::now(), bytes.to_vec()));
            assert!(super::drain(app, &mut keys, &wakes, &mut first).is_some());
        };
        let (head, tail) = click.as_bytes().split_at(7);
        read(&mut app, head);
        app.model.grab(grab);
        for _ in 0..filler {
            let _frame = framed(&mut app);
            read(&mut app, b"0");
        }
        assert_eq!(app.history.len(), kept, "{filler} bytes of one report");
        read(&mut app, tail);
        assert_eq!(app.model.selected(), Some("api"), "no retarget");
        assert_eq!(app.note().as_deref(), said);
        assert_eq!(app.model.drag().is_some(), kept == 0, "the drag");
    }
}

/// R3: the exit line names the way back, and outside tmux no tmux key.
#[test]
fn the_exit_line_names_tmux_keys_only_inside_tmux() {
    assert!(super::exit_hint(true).ends_with("(in a chat window, prefix h reopens it)"));
    assert_eq!(super::exit_hint(false), "ae app closed - run: ae app");
}

// ---- appsnappy: what the reader answered, and what is drawn of it.

/// A lane read for one session while another is selected changes only
/// what that session will show; the drawn lane stays the selection's.
#[test]
fn a_lane_for_another_session_never_changes_the_drawn_one() {
    let mut app = app(None);
    two(&mut app);
    app.answer(view("web", ID, 1, "web turn"));
    app.answer(view("ops", ID, 2, "ops turn"));
    let shown = framed(&mut app);
    assert!(shown.contains("web turn"), "{shown}");
    assert!(!shown.contains("ops turn"), "{shown}");
    let _ = app.model.key(Browse::Digit(2), &app.fleet, false);
    app.show();
    let shown = framed(&mut app);
    assert!(shown.contains("ops turn"), "the cached lane shows at once");
    assert!(!shown.contains("web turn"), "{shown}");
}

/// A never-read selection draws its header and ONE `loading` row,
/// never the lane of the session selected before it.
#[test]
fn a_cold_selection_says_loading_never_the_last_lane() {
    let mut app = app(None);
    two(&mut app);
    app.answer(view("web", ID, 1, "web turn"));
    let _ = app.model.key(Browse::Digit(2), &app.fleet, false);
    app.show();
    let shown = framed(&mut app);
    assert!(!shown.contains("web turn"), "{shown}");
    assert_eq!(shown.matches("loading").count(), 1, "{shown}");
    let header = shown.lines().next().expect("header row");
    assert!(header.contains("ops"), "{header}");
    app.answer(view("ops", ID, 2, "ops turn"));
    let shown = framed(&mut app);
    assert!(shown.contains("ops turn") && !shown.contains("loading"));
}

/// R6: the header names the selected session's recorded work dir beside
/// its name, `$HOME` as `~`, cut from the LEFT with an ellipsis when it
/// does not fit so the leaf stays; none recorded, or a selection the world
/// does not hold, draws none.
#[test]
fn the_header_shows_the_recorded_work_dir_home_shortened_cut_from_the_left() {
    let home = crate::doors::home().expect("a test HOME");
    let tail = "projects/clemens33/ae/crates/with/deeply/nested/directories/of/leaf-dir";
    let mut app = app(None);
    two(&mut app);
    app.world.sessions[0].work_dir = Some(format!("{}/{tail}", home.display()));
    app.show();
    let header = |app: &mut App, width| {
        let shown = framed_at(app, width);
        shown.lines().next().expect("header row").to_owned()
    };
    let wide = header(&mut app, 220);
    assert!(wide.contains(&format!("~/{tail}")), "{wide}");
    let narrow = header(&mut app, 100);
    assert!(
        narrow.contains('…') && narrow.contains("/leaf-dir"),
        "{narrow}"
    );
    assert!(
        !narrow.contains("~/projects/clemens33/ae/crates"),
        "{narrow}"
    );
    app.world.sessions[0].work_dir = None;
    app.show();
    let none = header(&mut app, 160);
    assert!(!none.contains('…') && !none.contains('~'), "{none}");
    // A selection the world holds no entry for guesses no dir and no ask;
    // the base is the viewer's fact and rides anyway.
    app.world = World::new(Timestamp::now(), Vec::new());
    app.show();
    assert!(app.overview.open.is_empty() && app.overview.dir.is_none());
    let unlisted = header(&mut app, 160);
    assert!(
        !unlisted.contains('~') && !unlisted.contains('/'),
        "{unlisted}"
    );
}

/// A lane older than the one shown, or read as an identity the session
/// no longer records, is dropped; a fleet read naming a new identity
/// takes the old lane with it.
#[test]
fn a_stale_or_replaced_lane_never_reinstalls() {
    let mut app = app(None);
    two(&mut app);
    app.answer(view("web", ID, 5, "fresh"));
    app.answer(view("web", ID, 4, "older"));
    assert!(framed(&mut app).contains("fresh"));
    app.answer(view("web", "another-id", 6, "replaced"));
    assert!(!framed(&mut app).contains("replaced"));
    let read = loader::FleetRead {
        dirs: app.dirs.clone(),
        ids: [("web", "new-id"), ("ops", ID)]
            .into_iter()
            .map(|(name, id)| (name.to_owned(), id.to_owned()))
            .collect(),
        sole: std::collections::BTreeMap::new(),
        world: app.world.clone(),
        facts: app.facts.clone(),
        needs: app.needs.clone(),
        fleet: app.fleet.clone(),
        pairs: std::collections::BTreeMap::new(),
        scanned: true,
    };
    app.answer(Answer::Fleet(read));
    let shown = framed(&mut app);
    assert!(
        !shown.contains("fresh") && shown.contains("loading"),
        "{shown}"
    );
    app.answer(view("web", ID, 7, "old identity, late"));
    assert!(!framed(&mut app).contains("old identity"));
    app.answer(view("web", "new-id", 8, "new identity"));
    assert!(framed(&mut app).contains("new identity"));
}

/// Before the fleet is read the first frame stands on the home name: its
/// header, `loading` in the list and in the chat, and no tab.
#[test]
fn the_first_frame_stands_before_any_read() {
    let mut app = App::new(Some("api".to_owned()), None, None, None);
    app.show();
    let shown = framed(&mut app);
    assert!(shown.lines().next().is_some_and(|row| row.contains("api")));
    assert_eq!(shown.matches("loading").count(), 2, "{shown}");
    assert!(!shown.contains("Overview"), "{shown}");
    assert!(!shown.contains("No sessions."), "{shown}");
}

/// A look read before any fleet read names its session survives the
/// first one: nothing is guessed before it, the read look after it.
#[test]
fn a_look_read_before_the_first_fleet_dresses_after_it() {
    let mut app = App::new(Some("api".to_owned()), None, None, None);
    assert_eq!(app.dressed(), (None, None), "nothing read: no colour");
    let off = crate::theme::Look::read("", "", "off", "");
    app.answer(Answer::Look {
        name: "api".to_owned(),
        look: Some(off),
        zone: None,
    });
    let read = loader::FleetRead {
        dirs: app.dirs.clone(),
        ids: std::iter::once(("api".to_owned(), ID.to_owned())).collect(),
        sole: std::collections::BTreeMap::new(),
        world: app.world.clone(),
        facts: app.facts.clone(),
        needs: app.needs.clone(),
        fleet: one_row(Some("api")),
        pairs: std::collections::BTreeMap::new(),
        scanned: true,
    };
    app.answer(Answer::Fleet(read));
    assert_eq!(app.dressed(), (Some(off), None), "home's read look stays");
}

/// One fleet read built from what `app` holds, naming `ids`.
fn read_of(app: &App, ids: &[(&str, &str)]) -> loader::FleetRead {
    loader::FleetRead {
        dirs: app.dirs.clone(),
        ids: ids
            .iter()
            .map(|(name, id)| ((*name).to_owned(), (*id).to_owned()))
            .collect(),
        sole: std::collections::BTreeMap::new(),
        world: app.world.clone(),
        facts: app.facts.clone(),
        needs: app.needs.clone(),
        fleet: app.fleet.clone(),
        pairs: app.pairs.clone(),
        scanned: true,
    }
}

/// A fleet read naming the same identity keeps a foreign session's last
/// lane and look; home keeps its own whatever the fleet names; a look
/// read before any fleet read survives the first one for a listed
/// session, and home's even unlisted.
#[test]
fn a_fleet_read_keeps_what_still_describes_its_session() {
    let mut app = app(Some("api"));
    two(&mut app);
    let drawn = Some(crate::theme::Look::DEFAULT);
    for name in ["web", "api"] {
        app.answer(Answer::Look {
            name: name.to_owned(),
            look: drawn,
            zone: None,
        });
    }
    app.ids.clear();
    app.answer(Answer::Fleet(read_of(&app, &[("web", ID), ("ops", ID)])));
    assert!(
        app.looks.contains_key("web"),
        "a listed session's early look"
    );
    assert!(app.looks.contains_key("api"), "home's early look, unlisted");
    app.answer(view("web", ID, 1, "web turn"));
    app.shown.insert(
        "api".to_owned(),
        super::Shown {
            id: "home-id".to_owned(),
            seq: 1,
            lane: Rc::default(),
            roster: None,
        },
    );
    app.answer(Answer::Fleet(read_of(&app, &[("web", ID), ("ops", ID)])));
    assert!(
        app.shown.contains_key("web"),
        "same identity keeps its lane"
    );
    assert!(app.looks.contains_key("web"), "and its look");
    assert!(app.shown.contains_key("api"), "home keeps its lane");
}

/// Routing in the app: a plain line takes the colead and its first body
/// line when the lead's `ae list` verdict is down; `@lead` is that line
/// only and the next plain line routes again.
#[test]
fn a_plain_line_routes_to_the_colead_while_the_lead_is_down() {
    let mut app = app(Some("api"));
    app.pair = vec!["lead".to_owned(), "colead".to_owned()];
    let mut entry = SessionEntry::new("api", Status::Running);
    let seat = |name: &str, alive| crate::digest::AgentEntry {
        name: name.to_owned(),
        alive: Some(alive),
        ..crate::digest::AgentEntry::default()
    };
    entry.agents = vec![seat("lead", false), seat("colead", true)];
    app.world = World::new(Timestamp::now(), vec![entry]);
    let route = |app: &App, raw: &[u8]| {
        let line = String::from_utf8_lossy(raw).into_owned();
        app.routed("api", raw, "lead".to_owned(), line)
    };
    assert_eq!(
        route(&app, b"hello"),
        (
            "colead".to_owned(),
            "(routed to you: lead not running)\nhello".to_owned(),
            Some("sent to colead (lead not running)".to_owned()),
        )
    );
    assert_eq!(
        route(&app, b"@lead hi"),
        ("lead".to_owned(), "@lead hi".to_owned(), None),
        "an explicit seat is that line only"
    );
}

/// The composer's address reads the route the send takes: a plain line
/// names the seat the router picks, a draft that starts `@seat` names
/// that seat, and a slash command or a refused `@` line names none.
#[test]
fn the_destination_is_the_route_the_send_takes() {
    let mut app = app(Some("api"));
    app.pair = vec!["lead".to_owned(), "colead".to_owned()];
    let mut entry = SessionEntry::new("api", Status::Running);
    let seat = |name: &str, alive| crate::digest::AgentEntry {
        name: name.to_owned(),
        alive: Some(alive),
        ..crate::digest::AgentEntry::default()
    };
    entry.agents = vec![seat("lead", false), seat("colead", true)];
    app.world = World::new(Timestamp::now(), vec![entry]);
    let dest = |app: &mut App, draft: &str| {
        app.draft = draft.to_owned();
        app.destination("api")
    };
    let to = |seat: &str, named| Some((seat.to_owned(), named));
    assert_eq!(dest(&mut app, ""), to("colead", false), "the lead is down");
    assert_eq!(dest(&mut app, "hello"), to("colead", false));
    assert_eq!(dest(&mut app, "@lead hi"), to("lead", true));
    assert_eq!(dest(&mut app, "@ghost hi"), None, "a refused line");
    assert_eq!(dest(&mut app, "/close"), None, "a command");
    let routed = app.routed("api", b"hello", "lead".to_owned(), "hello".to_owned());
    assert_eq!(
        Some(routed.0),
        dest(&mut app, "hello").map(|(seat, _)| seat)
    );
    app.pair = Vec::new();
    assert_eq!(dest(&mut app, "hello"), None, "no pair, no seat");
}

/// Routing weighs quota: with both seats free a plain line leaves a lead
/// pressed to 80% for a colead 20 points lower, the readings matched to
/// the pair by seat name; `@lead` is that line only, and a reading of
/// another incarnation weighs nothing.
#[test]
fn a_plain_line_routes_by_quota_while_both_seats_are_free() {
    let mut app = app(Some("api"));
    app.pair = vec!["lead".to_owned(), "colead".to_owned()];
    let mut entry = SessionEntry::new("api", Status::Running);
    let seat = |name: &str| crate::digest::AgentEntry {
        name: name.to_owned(),
        alive: Some(true),
        ..crate::digest::AgentEntry::default()
    };
    entry.agents = vec![seat("lead"), seat("colead")];
    app.world = World::new(Timestamp::now(), vec![entry]);
    app.ids.insert("api".to_owned(), ID.to_owned());
    let read = |id: &str, seats: [(&str, Option<f64>); 2]| Answer::Pressure {
        name: "api".to_owned(),
        id: id.to_owned(),
        pressure: seats.map(|(seat, used)| (seat.to_owned(), used)).to_vec(),
    };
    let route = |app: &App, raw: &[u8]| {
        let line = String::from_utf8_lossy(raw).into_owned();
        app.routed("api", raw, "lead".to_owned(), line)
    };
    app.answer(read(ID, [("colead", Some(40.0)), ("lead", Some(86.0))]));
    assert_eq!(
        route(&app, b"hello"),
        (
            "colead".to_owned(),
            "(routed to you: lead quota 86%)\nhello".to_owned(),
            Some("sent to colead (lead quota 86%)".to_owned()),
        )
    );
    assert_eq!(
        route(&app, b"@lead hi"),
        ("lead".to_owned(), "@lead hi".to_owned(), None),
        "an explicit seat outranks the quota"
    );
    app.answer(read(
        "another",
        [("lead", Some(95.0)), ("colead", Some(0.0))],
    ));
    assert_eq!(
        route(&app, b"hello").0,
        "lead",
        "another incarnation's reading"
    );
}

/// The lanes kept are the reader's kept order: a session past KEEP takes
/// its lane with it.
#[test]
fn a_lane_past_the_kept_order_is_dropped() {
    let mut app = app(None);
    let names: Vec<String> = (0..loader::KEEP + 4)
        .map(|at| format!("s{at:03}"))
        .collect();
    app.fleet = Fleet {
        rows: names
            .iter()
            .enumerate()
            .map(|(at, name)| row(name, at + 1, false))
            .collect(),
        home: None,
    };
    for name in &names {
        app.dirs.insert(name.clone(), PathBuf::from(name));
        app.ids.insert(name.clone(), ID.to_owned());
    }
    app.model = Model::new(&app.fleet);
    let (first, last, beyond) = (
        "s000".to_owned(),
        format!("s{:03}", loader::KEEP - 1),
        format!("s{:03}", loader::KEEP + 3),
    );
    for name in [&first, &last, &beyond] {
        app.answer(view(name, ID, 1, name));
    }
    app.evict();
    assert!(app.shown.contains_key(&first) && app.shown.contains_key(&last));
    assert!(!app.shown.contains_key(&beyond), "beyond KEEP");
}

/// Ruling 5: a write through the home composer asks the reader to read
/// home again; a key that writes nothing asks nothing.
#[test]
fn a_composer_write_asks_for_a_home_reread() {
    let root = Root::new("reread");
    let (mut app, _reader) = housed(&root);
    let (ask, asks) = std::sync::mpsc::channel();
    app.ask = Some(ask);
    app.fleet = one_row(Some("api"));
    app.model = Model::new(&app.fleet);
    let typed = writing(&mut app);
    asks.try_iter().for_each(drop);
    assert_eq!(app.compose(Key::Text(b"/close".to_vec()), typed), Some(()));
    assert!(asks.try_iter().next().is_none(), "typing writes nothing");
    assert_eq!(app.compose(Key::Enter, typed), Some(()));
    let rereads: Vec<String> = asks
        .try_iter()
        .filter_map(|request| match request {
            loader::Request::Reread(name) => Some(name),
            loader::Request::Focus(_)
            | loader::Request::Writing(_)
            | loader::Request::Peek(_)
            | loader::Request::Read(_)
            | loader::Request::Records(_)
            | loader::Request::Panel(_)
            | loader::Request::Ending(_)
            | loader::Request::Stats { .. }
            | loader::Request::Tools { .. }
            | loader::Request::Seat(_)
            | loader::Request::Settings { .. } => None,
        })
        .collect();
    assert_eq!(rereads, ["api"]);
}

/// A lone ESC completes once a wait ends with no keys: on the timeout
/// alone, and on answers alone.
#[test]
fn a_wait_with_no_keys_completes_a_lone_escape() {
    let (_wake, wakes) = std::sync::mpsc::channel();
    let quiet = Answer::Look {
        name: "web".to_owned(),
        look: None,
        zone: None,
    };
    let root = Root::new("lone-esc");
    for mut first in [None, Some(loader::Wake::Answer(Box::new(quiet)))] {
        let (mut app, _reader) = housed(&root);
        writing(&mut app);
        let mut keys = crate::console::input::Keys::app();
        let typed = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .expect("an instant a second ago");
        assert!(keys.feed(b"\x1b", typed).is_empty(), "ESC waits");
        assert_eq!(
            super::drain(&mut app, &mut keys, &wakes, &mut first),
            Some(true)
        );
        assert!(!app.composing(), "an Esc read before the lease browses");
    }
}

/// The foreign session `api` showing `turns` one-row turns, as read.
fn scrolled(turns: i64) -> App {
    let mut app = app(None);
    app.fleet = one_row(None);
    app.model = Model::new(&app.fleet);
    app.world = World::new(Timestamp::now(), vec![entry("api", Status::Running, None)]);
    app.dirs.insert("api".to_owned(), PathBuf::from("api"));
    app.ids.insert("api".to_owned(), ID.to_owned());
    app.answer(Answer::View(ViewRead {
        name: "api".to_owned(),
        id: ID.to_owned(),
        seq: 1,
        lane: Lane {
            items: (0..turns)
                .map(|at| said(&format!("turn {at}"), at))
                .collect(),
            coverage: Vec::new(),
        },
        needs: None,
        roster: None,
    }));
    app
}

/// `bytes` read at once on a 160x37 pane (27 chat rows), taken through
/// the loop's own drain and frame until no key is held.
fn replayed(app: &mut App, bytes: &[u8]) -> usize {
    let (_wake, wakes) = std::sync::mpsc::channel();
    let mut keys = crate::console::input::Keys::app();
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 37));
    app.frame(&mut buf);
    let mut first = Some(loader::Wake::Keys(Instant::now(), bytes.to_vec()));
    for _ in 0..64 {
        assert!(super::drain(app, &mut keys, &wakes, &mut first).is_some());
        app.frame(&mut buf);
        if app.deferred.is_empty() {
            return app.model.scroll_rows(30);
        }
    }
    panic!("keys still held");
}

/// A wheel notch down after page keys in the same read lands three rows
/// below where the pages went, as on a fully drawn lane, and never at the
/// edge of what the first frame happened to produce.
#[test]
fn a_notch_down_after_page_keys_keeps_the_pages() {
    let down = b"\x1b[<65;48;6M";
    let mut bytes = b"\x1b[5~".repeat(3);
    bytes.extend_from_slice(down);
    assert_eq!(replayed(&mut scrolled(160), &bytes), 90 - 3);
    // Past the true top the notch is taken from where the pages asked,
    // then bounded: the top, not three rows under it.
    let mut bytes = b"\x1b[5~".repeat(100);
    bytes.extend_from_slice(down);
    // The turns are from 2025, so the lane opens with a day separator row.
    let top = 80 * 3 - 1 + 1 - 30;
    assert_eq!(replayed(&mut scrolled(80), &bytes), top);
}

/// A notch landing exactly on the last row the frame drew is taken in
/// the same read: only one past it waits for the next frame.
#[test]
fn a_notch_onto_the_drawn_edge_is_taken_at_once() {
    let mut app = scrolled(160);
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 37));
    app.frame(&mut buf);
    assert!(!app.layout.complete);
    let edge = app.layout.max_scroll;
    assert_eq!((edge - 30) % 3, 0, "the edge is whole notches above a page");
    let (_wake, wakes) = std::sync::mpsc::channel();
    let mut keys = crate::console::input::Keys::app();
    let mut bytes = b"\x1b[5~".to_vec();
    bytes.extend(b"\x1b[<64;48;6M".repeat((edge - 30) / 3));
    let mut read = Some(loader::Wake::Keys(Instant::now(), bytes));
    assert!(super::drain(&mut app, &mut keys, &wakes, &mut read).is_some());
    assert!(app.deferred.is_empty(), "held at the edge");
    assert_eq!(app.model.scroll_rows(30), edge);
    let mut up = Some(loader::Wake::Keys(
        Instant::now(),
        b"\x1b[<64;48;6M".to_vec(),
    ));
    assert!(super::drain(&mut app, &mut keys, &wakes, &mut up).is_some());
    assert_eq!(app.deferred.len(), 1, "one past the edge waits");
}

/// Ruling 3: a selection shares its read lane rather than copying it.
#[test]
fn a_foreign_selection_shares_its_read_lane() {
    let mut app = app(Some("api"));
    two(&mut app);
    app.answer(view("web", ID, 1, "web turn"));
    assert_eq!(app.model.selected(), Some("web"));
    assert!(Rc::ptr_eq(&app.lane, &app.shown["web"].lane), "a copy");
}

/// Home's lane is its read with ae's own notices in it, in the order a
/// stable sort by time gives — the read's own order kept on a tie — and
/// it is built once per read and per notice, not on every show.
#[test]
fn homes_merge_is_built_once_per_read_and_notice() {
    let mut app = app(Some("api"));
    let base = Rc::new(Lane {
        items: vec![said("late", 30), said("early", 10), said("tie read", 20)],
        coverage: vec!["gap".to_owned()],
    });
    let api = |body, at| ("api".to_owned(), said(body, at));
    app.notices = vec![api("tie notice", 20), api("first", 0)];
    let bodies =
        |lane: &Lane| -> Vec<String> { lane.items.iter().map(|item| item.body.clone()).collect() };
    let merged = app.merge("api", Some(Rc::clone(&base)));
    assert_eq!(
        bodies(&merged),
        ["first", "early", "tie read", "tie notice", "late"]
    );
    assert_eq!(merged.coverage, ["gap"]);
    let again = app.merge("api", Some(Rc::clone(&base)));
    assert!(Rc::ptr_eq(&merged, &again), "rebuilt with nothing new");
    let read = Rc::new((*base).clone());
    let reread = app.merge("api", Some(Rc::clone(&read)));
    assert!(!Rc::ptr_eq(&again, &reread), "a new read is merged afresh");
    app.effects(vec![Effect::Print("said".to_owned())]);
    let noticed = app.merge("api", Some(Rc::clone(&read)));
    assert!(!Rc::ptr_eq(&reread, &noticed), "a new notice is merged");
    assert_eq!(bodies(&noticed).last().map(String::as_str), Some("said"));
    let unread = app.merge("api", None);
    assert_eq!(bodies(&unread).len(), 3, "notices alone before a read");
    assert!(Rc::ptr_eq(&unread, &app.merge("api", None)));
    // B2 NOTICES: a recorded session's notices draw only in its own lane,
    // an unrecorded one's in whichever lane is shown.
    app.dirs.insert("api".to_owned(), PathBuf::new());
    app.notice("gone", "gone's".to_owned());
    let web = app.merge("web", Some(Rc::clone(&read)));
    assert_eq!(bodies(&web), ["early", "tie read", "late", "gone's"]);
    app.dirs.insert("gone".to_owned(), PathBuf::new());
    app.noticed += 1;
    assert!(Rc::ptr_eq(&app.merge("web", Some(Rc::clone(&read))), &read));
    (0..=super::NOTICES).for_each(|n| app.notice("web", n.to_string()));
    let web = app.notices.iter().filter(|(of, _)| of == "web").count();
    assert_eq!(web, super::NOTICES, "capped per session");
}

/// The look of a session dresses the app only as that session's: the
/// selection's look never dresses home, a drawn home look wins.
#[test]
fn a_look_dresses_only_as_the_session_it_describes() {
    let mut app = app(Some("api"));
    let mut fleet = one_row(Some("api"));
    fleet.rows.push(row("web", 2, false));
    app.fleet = fleet;
    app.model = Model::new(&app.fleet);
    let drawn = crate::theme::Look::DEFAULT;
    app.answer(Answer::Look {
        name: "web".to_owned(),
        look: Some(drawn),
        zone: Some("+0200".to_owned()),
    });
    let _ = app.model.key(Browse::Digit(2), &app.fleet, false);
    assert_eq!(app.dressed(), (None, None), "home unread: no colour");
    let _ = app.model.key(Browse::Digit(1), &app.fleet, false);
    app.answer(Answer::Look {
        name: "api".to_owned(),
        look: None,
        zone: None,
    });
    assert_eq!(app.dressed().1, None, "web's zone never dresses home");
    let _ = app.model.key(Browse::Digit(2), &app.fleet, false);
    assert_eq!(app.dressed().1, Some("+0200"));
    app.answer(Answer::Look {
        name: "api".to_owned(),
        look: Some(drawn),
        zone: Some("-0500".to_owned()),
    });
    assert_eq!(app.dressed().1, Some("-0500"), "home's drawn look wins");
    for name in ["api", "web"] {
        app.answer(Answer::Look {
            name: name.to_owned(),
            look: None,
            zone: None,
        });
    }
    assert_eq!(
        app.dressed().0,
        Some(crate::theme::Look::DEFAULT),
        "read and lookless: the default drawn look"
    );
    let unread = App::new(None, None, None, None);
    assert_eq!(unread.dressed(), (None, None), "nothing read: no colour");
}

/// A selection made by the fleet itself — its first choice, or a
/// selected row leaving — is told to the reader like a key's.
#[test]
fn every_change_of_selection_is_told_to_the_reader() {
    let (ask, asks) = std::sync::mpsc::channel();
    let mut app = App::new(None, None, None, Some(ask));
    let told = || -> Vec<Option<String>> {
        asks.try_iter()
            .filter_map(|request| match request {
                loader::Request::Focus(name) => Some(name),
                loader::Request::Reread(_)
                | loader::Request::Writing(_)
                | loader::Request::Peek(_)
                | loader::Request::Read(_)
                | loader::Request::Records(_)
                | loader::Request::Panel(_)
                | loader::Request::Ending(_)
                | loader::Request::Stats { .. }
                | loader::Request::Tools { .. }
                | loader::Request::Seat(_)
                | loader::Request::Settings { .. } => None,
            })
            .collect()
    };
    app.focus();
    assert!(told().is_empty(), "nothing selected, nothing told");
    two(&mut app);
    app.focus();
    assert_eq!(told(), [Some("web".to_owned())]);
    app.focus();
    assert!(told().is_empty(), "once per change");
    app.fleet.rows.remove(0);
    app.model.reconcile(&app.fleet);
    app.focus();
    assert_eq!(told(), [Some("ops".to_owned())]);
}

// ---- settings overlay pins. Oracles: docs/app.md ## Settings and the
// ---- frozen tests/it/app_settings_spec.rs (live); these pin the seams
// ---- the live rig cannot see: request order, generation drops, merges.

fn settings_requests(asks: &std::sync::mpsc::Receiver<loader::Request>) -> Vec<(bool, u64)> {
    asks.try_iter()
        .filter_map(|request| match request {
            loader::Request::Settings { open, generation } => Some((open, generation)),
            loader::Request::Focus(_)
            | loader::Request::Reread(_)
            | loader::Request::Peek(_)
            | loader::Request::Read(_)
            | loader::Request::Records(_)
            | loader::Request::Panel(_)
            | loader::Request::Ending(_)
            | loader::Request::Stats { .. }
            | loader::Request::Tools { .. }
            | loader::Request::Seat(_)
            | loader::Request::Writing(_) => None,
        })
        .collect()
}

/// Open asks once per open on a fresh generation; close always tells.
#[test]
fn settings_open_asks_once_and_close_tells() {
    let (ask, asks) = std::sync::mpsc::channel();
    let mut app = App::new(Some("api".to_owned()), None, None, Some(ask));
    app.open_settings();
    app.open_settings();
    assert_eq!(settings_requests(&asks), [(true, 1)]);
    app.close_settings();
    assert_eq!(settings_requests(&asks), [(false, 1)]);
    app.open_settings();
    assert_eq!(settings_requests(&asks), [(true, 2)]);
}

/// R2: the reader captures the session it was last told is selected, so
/// an open tells the selection the UI shows first, and only a change.
#[test]
fn settings_open_tells_the_current_selection_first() {
    let (ask, asks) = std::sync::mpsc::channel();
    let mut app = App::new(None, None, None, Some(ask));
    let order = |asks: &std::sync::mpsc::Receiver<loader::Request>| -> Vec<String> {
        asks.try_iter()
            .map(|request| match request {
                loader::Request::Focus(name) => format!("focus {name:?}"),
                loader::Request::Settings { open, .. } => format!("settings {open}"),
                loader::Request::Reread(_)
                | loader::Request::Peek(_)
                | loader::Request::Read(_)
                | loader::Request::Records(_)
                | loader::Request::Panel(_)
                | loader::Request::Ending(_)
                | loader::Request::Stats { .. }
                | loader::Request::Tools { .. }
                | loader::Request::Seat(_)
                | loader::Request::Writing(_) => String::new(),
            })
            .collect()
    };
    two(&mut app);
    app.open_settings();
    assert_eq!(order(&asks), [r#"focus Some("web")"#, "settings true"]);
    app.close_settings();
    app.open_settings();
    assert_eq!(order(&asks), ["settings false", "settings true"]);
}

/// A quota scope that draws as one head line: facts, not an oracle.
fn quota_scope(title: &str) -> crate::quota::view::Scope {
    crate::quota::view::Scope {
        title: title.to_owned(),
        profiles: String::new(),
        account: None,
        only: None,
        windows: Vec::new(),
    }
}

/// Only the open generation paints; quota-only answers keep the rest.
#[test]
fn settled_merges_own_generation_and_drops_stale() {
    let mut app = app(Some("api"));
    app.open_settings();
    let quota = || vec![quota_scope("q")];
    let config = || settings::ConfigView::Rows {
        header: settings::ConfigHeader::default(),
        rows: Vec::new(),
    };
    let about = || settings::AboutFacts {
        tmux_version: "3.5".to_owned(),
        tmux_verdict: "ok",
        state_root: "/r".to_owned(),
        config_file: "/c".to_owned(),
        server: "ambient".to_owned(),
    };
    app.settled(99, quota(), Some(config()), Some(about()), None);
    assert!(app.settings_bodies.quota.is_none(), "stale dropped");
    app.settled(1, quota(), Some(config()), Some(about()), None);
    assert!(app.settings_bodies.quota.is_some(), "own paints");
    assert!(matches!(
        app.settings_bodies.config,
        settings::ConfigView::Rows { .. }
    ));
    app.settled(1, vec![quota_scope("q2")], None, None, None);
    assert_eq!(app.settings_bodies.quota.as_ref().expect("quota").len(), 1);
    assert_eq!(
        app.settings_bodies.quota.as_ref().expect("quota")[0].title,
        "q2"
    );
    assert!(
        matches!(
            app.settings_bodies.config,
            settings::ConfigView::Rows { .. }
        ),
        "quota-only keeps config"
    );
    assert!(
        app.settings_bodies.about.is_some(),
        "quota-only keeps about"
    );
    app.close_settings();
    app.settled(1, quota(), Some(config()), Some(about()), None);
    assert_eq!(
        app.settings_bodies.quota.as_ref().expect("quota")[0].title,
        "q2",
        "closed drops"
    );
}

/// Modal keys: Esc/q/s close and tell, Tab/1-3 switch, the rest dies.
#[test]
fn settings_keys_route_before_compose() {
    let (ask, asks) = std::sync::mpsc::channel();
    let mut app = App::new(Some("api".to_owned()), None, None, Some(ask));
    app.fleet = one_row(Some("api"));
    let at = Instant::now();
    app.open_settings();
    assert!(settings_key(&mut app, &Key::Text(b"x".to_vec()), at).is_some());
    assert!(app.model.settings_open(), "text swallowed");
    assert_eq!(app.model.settings().expect("open").tab, SettingsTab::Quota);
    assert!(settings_key(&mut app, &Key::Tab, at).expect("tab"));
    assert_eq!(app.model.settings().expect("open").tab, SettingsTab::Config);
    assert!(settings_key(&mut app, &Key::Text(b"4".to_vec()), at).is_some());
    let shown = app.model.settings().expect("open").tab;
    assert_eq!(
        shown,
        SettingsTab::Instructions,
        "digit 4 shows the fourth tab"
    );
    assert!(settings_key(&mut app, &Key::Text(b"5".to_vec()), at).is_some());
    assert_eq!(app.model.settings().expect("open").tab, SettingsTab::Keys);
    // digit 6: was swallowed (wildcard); now shows the sixth tab.
    assert!(settings_key(&mut app, &Key::Text(b"6".to_vec()), at).is_some());
    assert_eq!(
        app.model.settings().expect("open").tab,
        SettingsTab::Accounts,
        "digit 6 shows the sixth tab"
    );
    assert!(settings_key(&mut app, &Key::Tab, at).expect("statistics"));
    assert_eq!(
        app.model.settings().expect("open").tab,
        SettingsTab::Statistics
    );
    assert!(settings_key(&mut app, &Key::Tab, at).expect("wraps"));
    assert_eq!(app.model.settings().expect("open").tab, SettingsTab::Quota);
    assert!(settings_key(&mut app, &Key::Text(b"?".to_vec()), at).is_some());
    assert_eq!(app.model.settings().expect("open").tab, SettingsTab::Keys);
    assert!(settings_key(&mut app, &Key::Escape, at).expect("close"));
    assert!(!app.model.settings_open());
    assert_eq!(
        settings_requests(&asks).last(),
        Some(&(false, 1)),
        "close told"
    );
    app.open_settings();
    assert!(settings_key(&mut app, &Key::Text(b"q".to_vec()), at).expect("q closes"));
    assert!(!app.model.settings_open());
    app.open_settings();
    assert!(settings_key(&mut app, &Key::Text(b",!".to_vec()), at).expect(", closes"));
    assert!(!app.model.settings_open());
    app.open_settings();
    assert!(
        settings_key(&mut app, &Key::Interrupt, at).is_none(),
        "^C quits"
    );
}

fn ready(dir: &std::path::Path, name: &str) -> settings::Protocol {
    match settings::resolve_instructions(Some(dir), name) {
        settings::InstructionsView::Ready(protocol) => protocol,
        other => panic!("ready expected: {other:?}"),
    }
}

/// The rules are the render owner's generic rules, each roster seat names
/// the audiences its role receives, the winning file is named (an empty
/// winner masks, no row falls to the global) and a slot the context
/// classifies as neither lead nor worker is a named gap.
#[test]
fn resolve_instructions_is_the_render_owners_rules_and_each_seats_audiences() {
    let root = Root::new("instr-seats");
    let (global, overlay) = (root.0.join("global"), root.0.join("overlay"));
    std::fs::write(&global, "[prompt]\ninstructions = from global\n").expect("global");
    std::fs::write(&overlay, "[prompt]\ninstructions = from session\n").expect("overlay");
    let tail = format!(
        "work_dir=/w\nconfig={}\nlocal_config={}\nseat.spawned.1=helper\nwork_dir.spawned.1=/apart\nseat.extra=odd\n",
        global.display(),
        overlay.display()
    );
    let dir = session(&root, "api", &tail);
    let files = [global, overlay.clone()];
    let protocol = ready(&dir, "api");
    let custom = protocol.custom.as_ref().expect("instructions in force");
    assert_eq!(
        (custom.source, custom.file.as_str(), custom.text.as_str()),
        (
            settings::ConfigSource::Session,
            overlay.to_str().expect("utf-8"),
            "from session"
        )
    );
    let roles: Vec<_> = protocol.seats.iter().map(|seat| seat.role).collect();
    assert_eq!(roles, ["lead", "lead", "worker", "other"]);
    assert_eq!(protocol.rules, crate::render::generic_rules(&dir, &files));
    let received = |at: usize| protocol.seats[at].receives.as_ref().ok().copied();
    let (every, pair, workers) = (Audience::Every, Audience::LeadPair, Audience::Workers);
    assert_eq!(received(0), Some(&[every, pair][..]));
    assert_eq!(received(1), Some(&[every, pair][..]));
    assert_eq!(received(2), Some(&[every, workers][..]));
    let odd = protocol.seats[3].receives.as_ref();
    assert!(odd.expect_err("unknown slot").contains("unknown slot"));
    std::fs::write(&overlay, "[prompt]\ninstructions = \"\"\n").expect("overlay");
    let masked = ready(&dir, "api");
    assert_eq!(masked.custom, None, "the empty winner masks the global");
}

/// `spawn_instructions` is resolved apart from `instructions`, by the render
/// owner's winner rule: an empty winner of one key masks only that key.
#[test]
fn resolve_instructions_resolves_spawn_instructions_apart_with_its_own_mask() {
    let root = Root::new("instr-spawn");
    let (global, overlay) = (root.0.join("global"), root.0.join("overlay"));
    std::fs::write(
        &global,
        "[prompt]\ninstructions = common\nspawn_instructions = from global\n",
    )
    .expect("global");
    std::fs::write(
        &overlay,
        "[prompt]\nspawn_instructions = \"\"\"\nfrom\nsession\n\"\"\"\n",
    )
    .expect("overlay");
    let tail = format!(
        "work_dir=/w\nconfig={}\nlocal_config={}\n",
        global.display(),
        overlay.display()
    );
    let dir = session(&root, "api", &tail);
    let protocol = ready(&dir, "api");
    let spawn = protocol
        .spawn
        .as_ref()
        .expect("spawn instructions in force");
    assert_eq!(
        (spawn.source, spawn.file.as_str(), spawn.text.as_str()),
        (
            settings::ConfigSource::Session,
            overlay.to_str().expect("utf-8"),
            "from\nsession"
        )
    );
    assert_eq!(
        protocol.custom.as_ref().map(|custom| custom.text.as_str()),
        Some("common"),
        "the other key is untouched"
    );
    std::fs::write(&overlay, "[prompt]\nspawn_instructions = \"\"\n").expect("overlay");
    let masked = ready(&dir, "api");
    assert_eq!(masked.spawn, None, "the empty winner masks the global");
    assert!(masked.custom.is_some(), "and masks only its own key");
}

/// What cannot be rendered from is named: no records, a meta that cannot
/// be read, a torn config (the whole tab) and a slot that is no slot (the
/// one seat); a seat's work dir and name are no gap, the tab shows neither.
#[test]
fn resolve_instructions_names_every_gap() {
    let root = Root::new("instr-gaps");
    let gap = |dir: Option<&std::path::Path>, name| match settings::resolve_instructions(dir, name)
    {
        settings::InstructionsView::Gap(why) => why,
        other => panic!("gap expected: {other:?}"),
    };
    assert!(gap(None, "web").contains("web: no records"));
    let torn = root.0.join("torn");
    std::fs::write(&torn, "[prompt]\ninstructions = \"\"\"\nunclosed\n").expect("torn");
    let tail = format!("work_dir=/w\nconfig={}\n", torn.display());
    let why = gap(Some(&session(&root, "api", &tail)), "api");
    assert!(why.contains(torn.to_str().expect("utf-8")) && why.contains("unterminated"));
    let dir = session(
        &root,
        "web",
        "seat.spawned.2=-bad\nwork_dir.worker.0=relative\nseat.worker.0x=ok-one\nseat.spawned.=ok-two\n",
    );
    let seats = ready(&dir, "web").seats;
    let gaps: Vec<_> = seats
        .iter()
        .map(|seat| seat.receives.as_ref().err())
        .collect();
    assert!(gaps[..3].iter().all(Option::is_none), "{gaps:?}");
    assert!(
        gaps[3..]
            .iter()
            .all(|why| why.is_some_and(|why| why.contains("unknown slot"))),
        "{gaps:?}"
    );
}

/// Pins win over changed files; the overlay beats the recorded global;
/// global-only keys read the current global alone.
#[test]
fn resolve_config_pins_win_and_scopes_hold() {
    let root = Root::new("settings-resolve");
    let recorded = root.0.join("recorded");
    let overlay = root.0.join("overlay");
    let current = root.0.join("config");
    std::fs::write(
        &recorded,
        "[workspace]\nchat = on\nquota = on\nfleet_order = recorded-fleet\n",
    )
    .expect("recorded");
    std::fs::write(&overlay, "[workspace]\nchat = app\n").expect("overlay");
    std::fs::write(&current, "[workspace]\nfleet_order = current-fleet\n").expect("current");
    let dir = session(
        &root,
        "api",
        &format!(
            "config={}\nlocal_config={}\nquota=off\n",
            recorded.display(),
            overlay.display()
        ),
    );
    let settings::ConfigView::Rows { header, rows } =
        settings::resolve_config(Some(&dir), &current)
    else {
        panic!("rows expected");
    };
    let row = |key: &str| rows.iter().find(|row| row.key == key).expect(key);
    assert_eq!(header.global, recorded.display().to_string());
    assert_eq!(
        header.current_global.as_deref(),
        Some(current.display().to_string()).as_deref()
    );
    assert_eq!(row("quota").value, "off");
    assert_eq!(row("quota").source, settings::ConfigSource::Launch);
    assert_eq!(row("chat").value, "app");
    assert_eq!(row("chat").source, settings::ConfigSource::Session);
    assert_eq!(row("fleet_order").value, "current-fleet");
    assert_eq!(row("fleet_order").source, settings::ConfigSource::Global);
    assert!(row("fleet_order").global_only);
    assert_eq!(row("main").value, "(unset)");
    assert_eq!(row("main").source, settings::ConfigSource::Default);
}

/// A torn consulted file is one honest row naming the file and the reason.
#[test]
fn resolve_config_torn_file_is_one_honest_row() {
    let root = Root::new("settings-torn");
    let recorded = root.0.join("broken");
    std::fs::write(&recorded, "[prompt]\ninstructions = \"\"\"\nunclosed\n").expect("broken");
    let current = root.0.join("config");
    std::fs::write(&current, "[workspace]\nchat = on\n").expect("current");
    let dir = session(&root, "api", &format!("config={}\n", recorded.display()));
    let settings::ConfigView::Unreadable(why) = settings::resolve_config(Some(&dir), &current)
    else {
        panic!("one honest row expected");
    };
    assert!(
        why.contains(&recorded.display().to_string()),
        "names the file: {why}"
    );
    assert!(why.contains("unterminated"), "names the reason: {why}");
}

/// Missing files are no entries: everything falls through to defaults.
#[test]
fn resolve_config_missing_files_fall_to_defaults() {
    let root = Root::new("settings-missing");
    let current = root.0.join("config");
    let dir = session(&root, "api", "");
    let settings::ConfigView::Rows { header, rows } =
        settings::resolve_config(Some(&dir), &current)
    else {
        panic!("rows expected");
    };
    assert_eq!(header.global, current.display().to_string());
    assert!(header.current_global.is_none(), "not distinct");
    assert!(header.session.is_none(), "no overlay selected");
    assert_eq!(rows.len(), 22);
    let row = |key: &str| rows.iter().find(|row| row.key == key).expect(key);
    assert_eq!(row("chat").value, "on");
    assert_eq!(row("chat").source, settings::ConfigSource::Default);
    assert_eq!(row("auto_trust").value, "on");
    assert_eq!(row("auto_trust").source, settings::ConfigSource::Default);
}

/// No session at all: the tab is the current global alone, each row from
/// that file or its default, never a refusal.
#[test]
fn resolve_config_without_a_session_is_the_global_alone() {
    let root = Root::new("settings-nohome");
    let current = root.0.join("config");
    std::fs::write(&current, "[workspace]\nicons = off\n").expect("global config");
    let settings::ConfigView::Rows { header, rows } = settings::resolve_config(None, &current)
    else {
        panic!("rows expected");
    };
    assert_eq!(header.global, current.display().to_string());
    assert!(header.session.is_none());
    let row = |key: &str| rows.iter().find(|row| row.key == key).expect(key);
    assert_eq!(row("icons").value, "off");
    assert_eq!(row("icons").source, settings::ConfigSource::Global);
    assert_eq!(row("chat").source, settings::ConfigSource::Default);
}

/// A home session whose meta cannot be read names the meta file.
#[test]
fn resolve_config_unreadable_meta_names_file() {
    let root = Root::new("settings-nometa");
    let dir = root.0.join("sessions").join("ghost");
    std::fs::create_dir_all(&dir).expect("session dir");
    let current = root.0.join("config");
    let settings::ConfigView::Unreadable(why) = settings::resolve_config(Some(&dir), &current)
    else {
        panic!("one honest row expected");
    };
    assert!(
        why.contains(&dir.display().to_string()),
        "names the meta: {why}"
    );
}

/// Page keys move the drawn body height the frame set, not a constant.
#[test]
fn settings_page_keys_use_drawn_height() {
    let mut app = app(Some("api"));
    app.fleet = one_row(Some("api"));
    app.open_settings();
    app.model.set_settings_page_rows(5);
    app.model.key(Browse::PageDown, &app.fleet, false);
    assert_eq!(app.model.settings().expect("open").scroll, 5);
    app.model.key(Browse::PageDown, &app.fleet, false);
    assert_eq!(app.model.settings().expect("open").scroll, 10);
    app.model.key(Browse::PageUp, &app.fleet, false);
    assert_eq!(app.model.settings().expect("open").scroll, 5);
}

/// A batched chunk spells in byte order: `2s` selects session 2, then
/// opens; `qqs` quits before ever reaching s; `sq` opens and closes,
/// every transition carrying its request.
#[test]
fn settings_chunk_bytes_keep_order_and_requests() {
    let two = || Fleet {
        rows: vec![row("api", 1, true), row("web", 2, false)],
        home: Some("api".to_owned()),
    };
    let (ask, asks) = std::sync::mpsc::channel();
    let mut app = App::new(Some("api".to_owned()), None, None, Some(ask));
    app.fleet = two();
    assert!(browse(&mut app, &Key::Text(b"2,".to_vec()), Instant::now()).is_some());
    assert_eq!(app.model.selected(), Some("web"));
    assert!(app.model.settings_open());
    assert_eq!(settings_requests(&asks), [(true, 1)]);
    let (ask, asks) = std::sync::mpsc::channel();
    let mut app = App::new(Some("api".to_owned()), None, None, Some(ask));
    app.fleet = two();
    assert!(
        browse(&mut app, &Key::Text(b"qq,".to_vec()), Instant::now()).is_none(),
        "quits"
    );
    assert!(!app.model.settings_open());
    assert!(settings_requests(&asks).is_empty(), "no open asked");
    let (ask, asks) = std::sync::mpsc::channel();
    let mut app = App::new(Some("api".to_owned()), None, None, Some(ask));
    app.fleet = two();
    assert!(browse(&mut app, &Key::Text(b",q".to_vec()), Instant::now()).is_some());
    assert!(!app.model.settings_open());
    assert_eq!(settings_requests(&asks), [(true, 1), (false, 1)]);
}

/// Writing that starts mid-chunk drops the rest, read before the lease:
/// `is` starts writing on `i` and names the dropped `s`, never opening
/// settings (R-B3).
#[test]
fn settings_browse_chunk_resumes_composer_mid_chunk() {
    let root = Root::new("settings-is");
    let (ask, asks) = std::sync::mpsc::channel();
    let (mut app, _reader) = housed(&root);
    app.ask = Some(ask);
    app.fleet = one_row(Some("api"));
    app.model = Model::new(&app.fleet);
    let at = Instant::now();
    app.take(Reading::Owner, at);
    assert!(take_keys(&mut app, vec![(Key::Text(b"is".to_vec()), at)]).is_some());
    assert!(app.composing(), "writing started");
    assert!(!app.model.settings_open(), "never opened");
    assert_eq!(drafted(&app), "");
    let said = &app.notices.last().expect("the drop is said").1.body;
    assert_eq!(said, "dropped the keys typed before writing started");
    assert!(
        settings_requests(&asks).is_empty(),
        "zero settings requests"
    );
}

/// A modal close mid-chunk dispatches the rest as browse: `q2` closes
/// with its request, then selects session 2 with its lane.
#[test]
fn settings_modal_close_dispatches_rest_as_browse() {
    let (ask, asks) = std::sync::mpsc::channel();
    let mut app = App::new(Some("api".to_owned()), None, None, Some(ask));
    app.fleet = Fleet {
        rows: vec![row("api", 1, true), row("web", 2, false)],
        home: Some("api".to_owned()),
    };
    app.fleeted = true;
    app.open_settings();
    assert!(settings_key(&mut app, &Key::Text(b"q2".to_vec()), Instant::now()).is_some());
    assert!(!app.model.settings_open());
    assert_eq!(app.model.selected(), Some("web"));
    assert!(
        app.lane.coverage.iter().any(|line| line.contains("web")),
        "lane follows the selection"
    );
    assert_eq!(settings_requests(&asks).last(), Some(&(false, 1)));
}

/// A close mid-chunk resumes the suspended composer: the rest of the
/// run types into the kept draft whole — `s`, digits and multibyte runs
/// alike — with exactly one close request and no selection change.
#[test]
fn settings_modal_close_resumes_suspended_composer() {
    let root = Root::new("settings-resume");
    let (ask, asks) = std::sync::mpsc::channel();
    let (mut app, _reader) = housed(&root);
    app.ask = Some(ask);
    app.fleet = one_row(Some("api"));
    app.model = Model::new(&app.fleet);
    let at = writing(&mut app);
    assert_eq!(app.compose(Key::Text(b"kept".to_vec()), at), Some(()));
    app.open_settings();
    assert!(settings_key(&mut app, &Key::Text("qsé".as_bytes().to_vec()), at).is_some());
    assert!(!app.model.settings_open());
    assert!(app.composing(), "still writing");
    assert_eq!(
        drafted(&app),
        "keptsé",
        "rest typed whole into the kept draft"
    );
    assert_eq!(app.model.selected(), Some("api"), "no browse dispatch");
    assert_eq!(settings_requests(&asks), [(true, 1), (false, 1)]);
    app.open_settings();
    assert!(settings_key(&mut app, &Key::Text(b"q2".to_vec()), at).is_some());
    assert_eq!(drafted(&app), "keptsé2", "digits type too");
    assert_eq!(app.model.selected(), Some("api"));
    assert_eq!(
        settings_requests(&asks),
        [(true, 2), (false, 2)],
        "no extra requests"
    );
}

/// A gear click while the overlay is open closes it with its close
/// request and keeps the suspended composer: still writing, draft kept.
#[test]
fn settings_gear_click_closes_and_keeps_suspended_composer() {
    let root = Root::new("settings-gear-close");
    let (ask, asks) = std::sync::mpsc::channel();
    let (mut app, _reader) = housed(&root);
    app.ask = Some(ask);
    app.fleet = one_row(Some("api"));
    app.model = Model::new(&app.fleet);
    let at = writing(&mut app);
    assert_eq!(app.compose(Key::Text(b"kept".to_vec()), at), Some(()));
    app.open_settings();
    let shown = framed(&mut app);
    let keys = shown.lines().last().expect("keys row");
    let tail = keys.trim_end();
    assert!(
        tail.ends_with('\u{2699}') || tail.ends_with('*'),
        "gear drawn last: {tail:?}"
    );
    assert!(
        app.mouse(super::Mouse {
            kind: super::MouseKind::Click,
            column: 159,
            row: 44,
        }),
        "gear click closes"
    );
    assert!(!app.model.settings_open());
    assert_eq!(settings_requests(&asks), [(true, 1), (false, 1)]);
    assert!(app.composing(), "still writing");
    assert_eq!(drafted(&app), "kept");
}

/// The overlay's tab title and close label, still in the last frame's
/// layout after a key closed it, do nothing when pressed.
#[test]
fn settings_targets_left_in_the_last_frame_do_nothing_once_closed() {
    let root = Root::new("settings-stale-targets");
    let (mut app, _reader) = housed(&root);
    app.fleet = one_row(Some("api"));
    app.model = Model::new(&app.fleet);
    app.open_settings();
    let shown = framed(&mut app);
    let column = |row: usize, word: &str| {
        let line = shown.lines().nth(row).expect("drawn row");
        u16::try_from(line[..line.find(word).expect("drawn word")].chars().count()).expect("column")
    };
    let (config, close) = (column(1, "Config"), column(0, "close"));
    app.close_settings();
    for (column, row) in [(config, 1), (close, 0)] {
        let kind = super::MouseKind::Click;
        assert!(!app.mouse(super::Mouse { kind, column, row }));
    }
    assert!(!app.model.settings_open());
}

/// A drawn title press shows its tab now, without a reader answer to
/// repaint it later. A scrolled same-tab press resets; one at top is idle.
#[test]
fn settings_title_click_requests_its_frame_without_a_background_answer() {
    let mut app = App::new(None, None, None, None);
    app.fleeted = true;
    app.open_settings();
    let (_wake, wakes) = std::sync::mpsc::channel();
    let mut keys = crate::console::input::Keys::app();
    for (name, tab, scroll, redraw) in [
        ("Config", SettingsTab::Config, 3, true),
        ("Config", SettingsTab::Config, 3, true),
        ("Config", SettingsTab::Config, 0, false),
        ("About", SettingsTab::About, 3, true),
        ("Quota", SettingsTab::Quota, 3, true),
        ("Instructions", SettingsTab::Instructions, 3, true),
    ] {
        let shown = framed(&mut app);
        let titles = shown.lines().nth(1).expect("drawn tab titles");
        let byte = titles.find(name).expect("drawn title");
        let column = titles[..byte].chars().count() + name.chars().count() - 1;
        app.model.settings_wheel(scroll);
        let press = format!("\x1b[<0;{};2M", column + 1).into_bytes();
        let mut first = Some(loader::Wake::Keys(Instant::now(), press));
        assert_eq!(
            super::drain(&mut app, &mut keys, &wakes, &mut first),
            Some(redraw),
            "{name} click must request its own changed frame"
        );
        let open = app.model.settings().expect("overlay stays open");
        assert_eq!(open.tab, tab, "drawn title selects its tab");
        assert_eq!(open.scroll, 0, "title click resets like a key");
    }
}

/// A swallowed byte keeps the chunk's earlier redraw: `sx` opens on `s`
/// and swallows `x` modal, returning `Some(true)` with one open request.
#[test]
fn settings_chunk_swallowed_byte_keeps_earlier_redraw() {
    let (ask, asks) = std::sync::mpsc::channel();
    let mut app = App::new(Some("api".to_owned()), None, None, Some(ask));
    app.fleeted = true;
    app.fleet = Fleet {
        rows: vec![row("api", 1, true), row("web", 2, false)],
        home: Some("api".to_owned()),
    };
    assert_eq!(
        take_keys(&mut app, vec![(Key::Text(b",x".to_vec()), Instant::now())]),
        Some(true),
        "open redraw survives the swallowed byte"
    );
    assert!(app.model.settings_open());
    assert_eq!(settings_requests(&asks), [(true, 1)]);
}

/// A housed home plus a foreign row: session, tab, compose, border,
/// chat and gear geometry all drawn in one closed frame.
fn strip_app(root: &Root) -> (App, Reader) {
    let (mut app, reader) = housed(root);
    app.fleet = Fleet {
        rows: vec![row("api", 1, true), row("web", 2, false)],
        home: Some("api".to_owned()),
    };
    app.model = Model::new(&app.fleet);
    assert_eq!(app.model.selected(), Some("api"));
    app.take(Reading::Owner, Instant::now());
    app.world = World::new(
        Timestamp::now(),
        vec![
            entry("api", Status::Running, None),
            entry("web", Status::Running, None),
        ],
    );
    for name in ["api", "web"] {
        app.facts.insert(
            name.to_owned(),
            fleet::Facts::Seats {
                id: ID.to_owned(),
                agents: Vec::new(),
            },
        );
    }
    (app, reader)
}

/// The first cell of `needle` in `shown` whose drawn target matches.
fn strip_cell(
    shown: &str,
    app: &App,
    needle: &str,
    want: &str,
    matches: impl Fn(Option<super::draw::Hit>) -> bool,
) -> (usize, usize) {
    let click = |column: usize, row: usize| super::Mouse {
        kind: super::MouseKind::Click,
        column: u16::try_from(column).expect("frame column"),
        row: u16::try_from(row).expect("frame row"),
    };
    shown
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            line.match_indices(needle).find_map(|(byte, _)| {
                let column = line[..byte].chars().count();
                matches(app.layout.hit(click(column, row))).then_some((row, column))
            })
        })
        .unwrap_or_else(|| panic!("drawn {want}"))
}

/// Painting the open overlay strips what it covers: session and
/// compose targets, borders and chat geometry go, the gear target stays.
#[test]
fn settings_paint_strips_covered_targets_borders_and_chat() {
    let root = Root::new("settings-strip");
    let (mut app, _reader) = strip_app(&root);
    let shown = framed(&mut app);
    let click = |column: usize, row: usize| super::Mouse {
        kind: super::MouseKind::Click,
        column: u16::try_from(column).expect("frame column"),
        row: u16::try_from(row).expect("frame row"),
    };
    let (session_row, session_column) = strip_cell(&shown, &app, "web", "session target", |hit| {
        matches!(hit, Some(super::draw::Hit::Session(_)))
    });
    let (compose_row, compose_column) = strip_cell(&shown, &app, "api", "compose target", |hit| {
        matches!(hit, Some(super::draw::Hit::Compose))
    });
    assert!(app.layout.shows(Edge::Sidebar), "closed sidebar border");
    let chat_column = (0..160)
        .find(|&column| app.layout.in_chat(click(column, 10)))
        .expect("closed chat geometry");
    assert!(
        matches!(
            app.layout.hit(click(159, 44)),
            Some(super::draw::Hit::Settings)
        ),
        "closed gear target"
    );
    app.open_settings();
    let _ = framed(&mut app);
    assert!(
        app.layout.hit(click(session_column, session_row)).is_none(),
        "covered session target gone"
    );
    assert!(
        app.layout.hit(click(compose_column, compose_row)).is_none(),
        "covered compose target gone"
    );
    assert!(!app.layout.shows(Edge::Sidebar), "covered border gone");
    assert!(
        !app.layout.in_chat(click(chat_column, 10)),
        "covered chat geometry gone"
    );
    assert!(
        matches!(
            app.layout.hit(click(159, 44)),
            Some(super::draw::Hit::Settings)
        ),
        "gear target stays"
    );
}

/// The gear is the toggle: at 80x19 the version and the gear end the
/// keys row, and clicking the gear cell opens Settings.
#[test]
fn settings_gear_cell_beside_the_version_opens_settings() {
    let root = Root::new("settings-gear-only");
    let (ask, asks) = std::sync::mpsc::channel();
    let (mut app, _reader) = housed(&root);
    app.ask = Some(ask);
    app.fleet = one_row(Some("api"));
    app.model = Model::new(&app.fleet);
    app.take(Reading::Owner, Instant::now());
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 19));
    app.frame(&mut buf);
    let shown = text(&buf);
    let keys = shown.lines().last().expect("keys row");
    let tail = keys.trim_end();
    let version = crate::shape::version_line();
    assert!(
        tail.ends_with(&format!("{version} \u{2699}")) || tail.ends_with(&format!("{version} *")),
        "version, then the gear last: {tail:?}"
    );
    assert!(
        app.mouse(super::Mouse {
            kind: super::MouseKind::Click,
            column: 79,
            row: 18,
        }),
        "gear cell opens"
    );
    assert!(app.model.settings_open());
    assert_eq!(settings_requests(&asks), [(true, 1)]);
}

/// Below the floor in either dimension an open overlay keeps the
/// closed fallback: the literal row, blank rest, no panel, no targets.
#[test]
fn settings_below_floor_either_dimension_keeps_fallback() {
    for (width, height) in [(39u16, 19u16), (89u16, 7u16)] {
        let root = Root::new("settings-floor");
        let (mut app, _reader) = housed(&root);
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        app.open_settings();
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        app.frame(&mut buf);
        let shown = text(&buf);
        let expected = format!("ae app needs at least 40x8 (now {width}x{height})");
        let mut rows = shown.lines();
        assert_eq!(
            rows.next().expect("fallback row").trim_end(),
            expected,
            "{width}x{height} literal"
        );
        assert!(
            rows.all(|row| row.trim().is_empty()),
            "{width}x{height} rest blank"
        );
        for tab in ["Settings", "Quota", "Config", "About"] {
            assert!(!shown.contains(tab), "{width}x{height} no {tab}");
        }
        assert_eq!(app.layout.settings_page_rows, 0, "{width}x{height} no body");
        assert_eq!(
            app.layout.settings_max_scroll, 0,
            "{width}x{height} no scroll"
        );
        for (column, row) in [(0, 0), (width / 2, height / 2), (width - 1, height - 1)] {
            assert!(
                app.layout
                    .hit(super::Mouse {
                        kind: super::MouseKind::Click,
                        column,
                        row,
                    })
                    .is_none(),
                "{width}x{height} no target at {column}x{row}"
            );
        }
    }
}

/// The panel grounds only the rows above its footer: with a drawn
/// DARCULA look the closed hint row is ink over the chat, and the open
/// overlay paints base over the covered rows while the footer stays ink.
#[test]
fn settings_panel_grounds_covered_rows_and_keeps_keys_ink() {
    use ratatui_core::style::Color;
    let (ink, base) = (Color::Rgb(0x2B, 0x2B, 0x2B), Color::Rgb(0x31, 0x33, 0x35));
    let root = Root::new("settings-grounds");
    let (mut app, _reader) = housed(&root);
    app.fleet = one_row(Some("api"));
    app.model = Model::new(&app.fleet);
    let darcula = crate::theme::Look::read("on", "darcula", "on", "on");
    app.answer(Answer::Look {
        name: "api".to_owned(),
        look: Some(darcula),
        zone: None,
    });
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
    app.frame(&mut buf);
    for x in 45..160 {
        assert_eq!(buf[(x, 44)].bg, ink, "closed hint row ink at {x}");
    }
    app.open_settings();
    app.frame(&mut buf);
    let shown = text(&buf);
    assert!(
        shown
            .lines()
            .next()
            .is_some_and(|line| line.contains("Settings")),
        "open title drawn"
    );
    assert!(
        shown.lines().last().is_some_and(|line| {
            let tail = line.trim_end();
            tail.ends_with('\u{2699}') || tail.ends_with('*')
        }),
        "gear present"
    );
    for x in 0..160 {
        assert_eq!(buf[(x, 44)].bg, ink, "open keys row stays ink at {x}");
    }
    for (x, y) in [(0, 0), (80, 20), (159, 43)] {
        assert_eq!(buf[(x, y)].bg, base, "covered row base at ({x},{y})");
    }
}

/// At the widest panes the title still draws: `width + LEFT` would
/// overflow u16 where `width - LEFT` fits.
#[test]
fn settings_title_draws_at_terminal_max_width() {
    for width in [65534u16, 65535u16] {
        let root = Root::new("settings-max-width");
        let (mut app, _reader) = housed(&root);
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        app.open_settings();
        let mut buf = Buffer::empty(Rect::new(0, 0, width, 8));
        app.frame(&mut buf);
        let shown = text(&buf);
        let mut rows = shown.lines();
        assert!(
            rows.next().is_some_and(|line| line.contains("Settings")),
            "{width} title row"
        );
        let tabs = rows.next().expect("tab row");
        for tab in ["Quota", "Config", "About"] {
            assert!(tabs.contains(tab), "{width} {tab}");
        }
        assert!(
            shown
                .lines()
                .last()
                .is_some_and(|line| line.contains("close")),
            "{width} close hint"
        );
    }
}

/// An empty ready quota reads honestly: cold shows loading, the empty
/// answer shows the no-scopes message instead.
#[test]
fn settings_empty_ready_quota_reads_honest() {
    let mut app = app(Some("api"));
    app.open_settings();
    let cold = framed(&mut app);
    assert!(cold.contains("loading"), "cold loads:\n{cold}");
    app.answer(Answer::Settings {
        generation: 1,
        quota: Vec::new(),
        config: None,
        about: None,
        instructions: None,
        live_seats: None,
    });
    let shown = framed(&mut app);
    assert!(
        shown.contains("no quota scopes read"),
        "honest empty:\n{shown}"
    );
    assert!(!shown.contains("loading"), "ready, not loading:\n{shown}");
}

/// The paint resets covered cells whole and clamps a past-the-end scroll
/// to the last page instead of an emptied body.
#[test]
fn settings_paint_resets_cells_and_clamps_bottom() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10));
    for y in 0..10 {
        for x in 0..40 {
            buf[(x, y)].set_symbol("X");
        }
    }
    let rows = |count: usize| {
        (0..count)
            .map(|at| quota_scope(&format!("row-{at}")))
            .collect::<Vec<_>>()
    };
    let bodies = settings::SettingsBodies {
        quota: Some(rows(10)),
        config: settings::ConfigView::Loading,
        about: None,
        ..settings::SettingsBodies::default()
    };
    let mut layout = draw::Layout::default();
    draw::paint_settings(SettingsTab::Quota, 99, &bodies, None, &mut buf, &mut layout);
    let frame = text(&buf);
    let rows: Vec<&str> = frame.lines().collect();
    assert!(
        !rows[..9].iter().any(|row| row.contains('X')),
        "cells reset"
    );
    assert_eq!(rows[3].trim(), "row-4", "clamped to the last page");
    assert_eq!(rows[8].trim(), "row-9");
    // Ten heads plus the one column-header line above them.
    assert_eq!(layout.settings_max_scroll, 5);
    assert_eq!(layout.settings_page_rows, 6);
}

/// I13: re-selecting the open tab with its digit shows it from the
/// first row again, even when already selected.
#[test]
fn reselecting_the_open_tab_restores_its_first_row() {
    let mut app = app(Some("api"));
    app.open_settings();
    let quota: Vec<crate::quota::view::Scope> = (0..60)
        .map(|at| quota_scope(&format!("qrow-{at:02}")))
        .collect();
    app.answer(Answer::Settings {
        generation: 1,
        quota,
        config: None,
        about: None,
        instructions: None,
        live_seats: None,
    });
    let first_body_row = |shown: &str| {
        shown
            .lines()
            .find(|row| row.contains("qrow-"))
            .expect("a body row")
            .to_owned()
    };
    let top = first_body_row(&framed(&mut app));
    assert!(top.contains("qrow-00"), "starts at the top:\n{top}");
    let at = Instant::now();
    // Two lines: the first body line is the column header, not a `qrow-`.
    assert!(take_keys(&mut app, vec![(Key::Text(b"jj".to_vec()), at)]).is_some());
    let scrolled = first_body_row(&framed(&mut app));
    assert_ne!(scrolled, top, "j scrolls off the top");
    assert!(scrolled.contains("qrow-01"), "one row down:\n{scrolled}");
    assert!(take_keys(&mut app, vec![(Key::Text(b"1".to_vec()), at)]).is_some());
    assert_eq!(
        first_body_row(&framed(&mut app)),
        top,
        "reselect restores the first row"
    );
}

/// I14: the frame settles an overscrolled overlay offset to the drawn
/// maximum, so the next key up visibly moves instead of redrawing the
/// same bottom page.
#[test]
fn an_overscrolled_overlay_settles_before_the_next_key() {
    let mut app = app(Some("api"));
    app.open_settings();
    let quota: Vec<crate::quota::view::Scope> = (0..10)
        .map(|at| quota_scope(&format!("qrow-{at:02}")))
        .collect();
    app.answer(Answer::Settings {
        generation: 1,
        quota,
        config: None,
        about: None,
        instructions: None,
        live_seats: None,
    });
    let body_ends = |app: &mut App| {
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10));
        app.frame(&mut buf);
        let body: Vec<String> = text(&buf)
            .lines()
            .filter(|row| row.contains("qrow-"))
            .map(str::to_owned)
            .collect();
        (
            body.first().expect("a first body row").clone(),
            body.last().expect("a last body row").clone(),
        )
    };
    let (top, _) = body_ends(&mut app);
    assert!(top.contains("qrow-00"), "starts at the top:\n{top}");
    let at = Instant::now();
    let past_end = vec![(Key::Text(vec![b'j'; 20]), at)];
    assert!(take_keys(&mut app, past_end).is_some());
    let (top, last) = body_ends(&mut app);
    assert!(top.contains("qrow-04"), "settled bottom page:\n{top}");
    assert!(last.contains("qrow-09"), "last row visible:\n{last}");
    let up = vec![(Key::Text(b"k".to_vec()), at)];
    assert!(take_keys(&mut app, up).is_some());
    let (top, last) = body_ends(&mut app);
    assert!(top.contains("qrow-03"), "one line up:\n{top}");
    assert!(last.contains("qrow-08"), "page moved:\n{last}");
}

/// R-B4: revocation never turns the rest of a refused stream into browse keys.
#[test]
fn a_held_entry_stays_held_when_home_is_revoked() {
    let root = Root::new("revoke-held");
    let dir = root.0.join("sessions").join("api");
    let (mut app, _reader) = housed(&root);
    app.take(Reading::Owner, Instant::now());
    // S2: a busy lease refuses no entry; a pair the meta cannot name does.
    meta(&root, ID, "bad.name");
    app.write(true);
    assert!(app.held.is_some(), "GUARD refused entry is HELD");
    std::fs::remove_file(dir.join("meta")).expect("home becomes unproven");
    let why = "home revoked";
    app.take(Reading::NotOwner(why.to_owned()), Instant::now());
    assert_eq!(app.held.as_deref(), Some(why), "revocation updates HELD");
    let q = vec![(Key::Text(b"q".to_vec()), Instant::now())];
    assert_eq!(take_keys(&mut app, q), Some(false), "q stays swallowed");
    assert_eq!(app.held.as_deref(), Some(why));
    let retry = vec![(Key::Enter, Instant::now())];
    assert_eq!(take_keys(&mut app, retry), Some(true));
    assert!(app.held.is_some(), "unproven retry remains HELD");
}

/// R-B5/R-B7: a revoked writer releases, keeps its draft, and holds later text.
#[test]
fn a_revoked_writer_releases_the_lease_and_holds_its_kept_draft() {
    let root = Root::new("revoke-write");
    let dir = root.0.join("sessions").join("api");
    let (mut app, _reader) = housed(&root);
    let at = writing(&mut app);
    assert_eq!(app.compose(Key::Text(b"keep me".to_vec()), at), Some(()));
    std::fs::remove_file(dir.join("meta")).expect("home becomes unproven");
    let why = "home revoked";
    app.take(Reading::NotOwner(why.to_owned()), Instant::now());
    assert!(!app.composing(), "revocation ends writing");
    drop(
        crate::store::open(&dir)
            .console_writer()
            .expect("lease released"),
    );
    assert_eq!(app.held.as_deref(), Some(why), "writer lands HELD");
    assert_eq!(
        app.inputs.values().next().expect("input").draft(),
        "keep me"
    );
    let q = vec![(Key::Text(b"q".to_vec()), Instant::now())];
    assert_eq!(take_keys(&mut app, q), Some(false), "q stays swallowed");
    assert_eq!(
        app.inputs.values().next().expect("input").draft(),
        "keep me"
    );
    let retry = vec![(Key::Enter, Instant::now())];
    assert_eq!(take_keys(&mut app, retry), Some(true));
    assert!(app.held.is_some(), "unproven retry remains HELD");
    assert_eq!(
        app.inputs.values().next().expect("input").draft(),
        "keep me"
    );
}

/// R-B5/R-B7: admission revocation keeps even the line Enter consumed.
#[test]
fn an_admission_revocation_keeps_the_entered_line_only_in_memory() {
    let root = Root::new("revoke-enter");
    let dir = root.0.join("sessions").join("api");
    let (mut app, _reader) = housed(&root);
    let at = writing(&mut app);
    assert_eq!(app.compose(Key::Text(b"keep me".to_vec()), at), Some(()));
    meta(&root, &ID.replace("1234", "bbbb"), "colead");
    assert_eq!(app.compose(Key::Enter, at), Some(()));
    assert!(app.deliver(), "the queued ask meets the admission");
    assert!(!app.composing(), "refused admission releases the lease");
    drop(crate::store::open(&dir).console_writer().expect("released"));
    assert_eq!(
        app.inputs.values().next().expect("input").draft(),
        "keep me"
    );
    assert_eq!(
        app.held.as_deref(),
        Some("the session was replaced or renamed")
    );
    let q = vec![(Key::Text(b"q".to_vec()), Instant::now())];
    assert_eq!(take_keys(&mut app, q), Some(false), "q stays swallowed");
    assert_eq!(super::submit::restore(&dir), super::submit::Draft::Nothing);
    assert_eq!(
        super::submit::recover(&app.recovery.record(&dir).expect("own lifetime")),
        super::submit::Draft::Nothing,
        "no own recovery bytes were published"
    );
    assert!(matches!(
        crate::store::open(&dir).events_source(),
        crate::store::SourceRead::Absent
    ));
    let body = std::fs::remove_dir(dir.join("messages")).map_err(|why| why.kind());
    assert_eq!(body, Err(std::io::ErrorKind::NotFound), "no body written");
}

/// #248 P1b-2 Batch A amendment (lead ruling e581edb4): an app writer
/// whose session was replaced under its name re-enters the new
/// incarnation with no refusal, publishes its draft there, and the old
/// incarnation's record writes nothing into the replacement. `uuid` is
/// the replacement's: a new one, or the same one in a new directory.
fn reentered(tag: &str, uuid: &str) {
    let root = Root::new(tag);
    let dir = root.0.join("sessions").join("api");
    let (mut app, reader) = housed(&root);
    writing(&mut app);
    let old = app.recovery.record(&dir).expect("own lifetime");
    old.publish(b"before").expect("the bound record publishes");
    let again = app.recovery.record(&dir).expect("own lifetime");
    assert!(
        std::sync::Arc::ptr_eq(&old, &again),
        "{tag}: a current record is kept"
    );
    std::fs::rename(dir.join("meta"), root.0.join("meta")).expect("meta aside");
    let unknown = app.recovery.record(&dir).expect("own lifetime");
    assert!(
        std::sync::Arc::ptr_eq(&old, &unknown),
        "{tag}: an unknown record is kept"
    );
    std::fs::rename(root.0.join("meta"), dir.join("meta")).expect("meta back");
    drop((again, unknown));
    app.write(false);
    std::fs::rename(&dir, root.0.join("gone")).expect("unregistered");
    std::fs::create_dir_all(&dir).expect("a same-name replacement");
    meta(&root, uuid, "colead");
    refold(&mut app, &reader, &root, &[("api", Status::Running)]);
    let seen = app.notices.len();
    writing(&mut app);
    let said: Vec<&str> = app.notices[seen..]
        .iter()
        .map(|(_, n)| n.body.as_str())
        .collect();
    assert!(
        !said
            .iter()
            .any(|body| body.contains("refused") || body.contains("not restored")),
        "{tag}: re-entry surfaces no refusal: {said:?}"
    );
    assert!(app.held.is_none(), "{tag}: {:?}", app.held);
    assert_eq!(
        drafted(&app),
        "",
        "{tag}: nothing of the old incarnation is restored"
    );
    let fresh = app
        .recovery
        .record(&dir)
        .expect("a record of the new incarnation");
    fresh
        .publish(b"after")
        .expect("the draft publishes in the new incarnation");
    assert!(
        old.publish(b"stale").is_err(),
        "{tag}: the old record refuses"
    );
    let _ = old.clear();
    assert_eq!(
        super::submit::recover(&fresh),
        super::submit::Draft::Kept(b"after".to_vec()),
        "{tag}: nothing written into the replacement by the old record"
    );
}

#[test]
fn a_writer_reentering_a_replaced_session_binds_the_new_incarnation() {
    reentered("reenter", &ID.replace("1234", "cccc"));
}

/// A replacement keeping the UUID is told apart by its directory alone.
#[test]
fn a_writer_reentering_a_same_uuid_replacement_binds_the_new_directory() {
    reentered("reenter-dir", ID);
}

/// #248 P1b-2 Batch A amendment 2 (lead ruling on 1e1b8b61): a draft
/// mutation guard that is not a regular file refuses the entry at once,
/// naming its path, and the writer sees why; a regular guard enters.
#[test]
fn a_nonregular_draft_mutation_guard_refuses_the_entry_by_name() {
    let root = Root::new("guard-dir");
    let guard = root
        .0
        .join("sessions")
        .join("api")
        .join(".console-draft-mutation.lock");
    std::fs::create_dir(&guard).expect("the guard is a directory");
    let (mut app, _reader) = housed(&root);
    app.take(Reading::Owner, Instant::now());
    let started = Instant::now();
    app.write(true);
    let took = started.elapsed();
    assert!(!app.composing(), "a non-regular guard refuses the entry");
    let why = app.held.clone().unwrap_or_default();
    assert!(
        why.contains(&guard.display().to_string())
            && why.contains("draft mutation guard")
            && !why.contains("lifetime lock"),
        "the writer is told the guard, not a busy lifetime lock: {why:?}"
    );
    assert!(
        took < Duration::from_secs(4),
        "refused at once, not after a wait: {took:?}"
    );
    std::fs::remove_dir(&guard).expect("the guard dir removed");
    app.write(false);
    writing(&mut app);
    assert!(app.held.is_none(), "{:?}", app.held);
}

/// An app on `api` whose lane holds one `said` turn for each of `bodies`,
/// framed once so the keys and clicks have a layout to act on.
fn turned(bodies: &[&str]) -> App {
    let mut app = app(Some("api"));
    app.fleet = one_row(Some("api"));
    app.model = Model::new(&app.fleet);
    let items = (bodies.iter().zip(0..)).map(|(body, at)| said(body, at));
    app.lane = Rc::new(Lane {
        items: items.collect(),
        coverage: Vec::new(),
    });
    let _ = framed(&mut app);
    app
}

fn key_of(app: &App, at: usize) -> u64 {
    super::lane::key(&app.lane.items[at])
}

/// A real Enter on a writable selection starts writing (the Keys table);
/// it folds or unfolds instead only when a MARKED turn has a fold row, and
/// never once the composer is already taking keys.
#[test]
fn enter_toggles_only_a_marked_turn_that_has_a_fold_row() {
    let root = Root::new("fold-enter");
    let long = (1..=32).map(|n| format!("row {n}")).collect::<Vec<_>>();
    let long = long.join("\n");
    let long = long.as_str();
    // One real Enter on the writable home `api`, its newest turn marked
    // or not, the composer already open or not: the state it left, and
    // the frame.
    let pressed = |bodies: &[&str], marked: bool, open: bool| {
        let (mut app, _reader) = housed(&root);
        app.take(Reading::Owner, Instant::now());
        if let Answer::View(mut read) = view("api", ID, 1, "") {
            read.lane.items = bodies.iter().zip(0..).map(|(b, at)| said(b, at)).collect();
            app.answer(Answer::View(read));
        }
        if open {
            writing(&mut app);
        }
        let _ = framed(&mut app);
        assert!(!marked || app.step_turn(false), "the newest turn is marked");
        let folds = framed(&mut app).contains("more lines");
        assert_eq!(folds, bodies.contains(&long), "folded");
        let mark = app.model.turn();
        assert!(take_keys(&mut app, vec![(Key::Enter, Instant::now())]).is_some());
        let queued = (app.queued.len(), app.expanded.len());
        let kept = app.model.turn() == mark;
        (
            (app.composing(), drafted(&app), queued, kept),
            framed(&mut app),
        )
    };
    let writes = (true, String::new(), (0, 0), true);
    for (bodies, marked, open) in [
        (&["short"][..], true, false),
        (&[long, "short"], true, false),
        (&[long], false, false),
        (&[long], true, true),
    ] {
        let (state, _) = pressed(bodies, marked, open);
        assert_eq!(state, writes, "{bodies:?} marked {marked} open {open}");
    }
    let (state, shown) = pressed(&[long], true, false);
    assert_eq!(state, (false, String::new(), (0, 1), true), "toggles");
    assert!(shown.contains("row 32") && !pressed(&[long], true, true).1.contains("row 32"));
}

/// `[` and `]` walk the produced turns: none marked takes the newest,
/// each press steps one, and an edge is silent and keeps the mark.
#[test]
fn the_turn_keys_walk_the_produced_turns_and_keep_the_mark_at_an_edge() {
    let mut app = turned(&["one", "two", "three"]);
    assert!(app.step_turn(false), "none marked: the newest shown");
    assert_eq!(app.model.turn(), Some(key_of(&app, 2)));
    assert!(!app.step_turn(false), "no newer turn: silent");
    assert_eq!(app.model.turn(), Some(key_of(&app, 2)));
    assert!(app.step_turn(true) && app.step_turn(true));
    assert_eq!(app.model.turn(), Some(key_of(&app, 0)));
    assert!(!app.step_turn(true), "no older turn: silent");
    assert_eq!(app.model.turn(), Some(key_of(&app, 0)));
    assert!(app.step_turn(false));
    assert_eq!(app.model.turn(), Some(key_of(&app, 1)));
    let mut empty = turned(&[]);
    assert!(
        !empty.step_turn(true) && !empty.step_turn(false),
        "no turns"
    );
    assert_eq!(empty.model.turn(), None);
}

/// Every refusal before a copy is a plain hint; outside tmux the copy goes
/// to the terminal and is not claimed to have arrived. The stale mark is
/// cleared, the others kept.
#[test]
fn a_copy_that_cannot_reach_tmux_says_why_on_the_hint_row() {
    let mut app = turned(&["one", ""]);
    let hint = |app: &mut App| {
        app.copy();
        app.flash.take().expect("a hint").0
    };
    assert_eq!(hint(&mut app), "no turn marked - click one or press [ ]");
    app.model.set_turn(Some(0));
    assert_eq!(
        hint(&mut app),
        "that turn changed or left the lane - mark it again"
    );
    assert_eq!(app.model.turn(), None, "the stale mark is cleared");
    app.model.set_turn(Some(key_of(&app, 1)));
    assert_eq!(hint(&mut app), "nothing to copy: that turn has no text");
    assert_eq!(app.model.turn(), Some(key_of(&app, 1)), "the mark is kept");
    app.model.set_turn(Some(key_of(&app, 0)));
    assert_eq!(
        hint(&mut app),
        "sent 1 line to the terminal clipboard - unconfirmed"
    );
    assert_eq!(app.clip.as_deref(), Some("one"), "staged for the terminal");
}

/// The terminal clipboard takes 65536 UTF-8 bytes; one more is refused by
/// name before anything is staged, and nothing is ever cut.
#[test]
fn the_terminal_clipboard_takes_65536_bytes_and_refuses_one_more() {
    let mut app = app(Some("api"));
    let fits = format!("{}a", "\u{4f60}".repeat(21845));
    assert_eq!(fits.len(), 65536);
    assert!(app.ship(&fits, "1 line").starts_with("sent 1 line "));
    assert_eq!(app.clip.as_deref(), Some(fits.as_str()));
    app.clip = None;
    let said = app.ship(&format!("{fits}b"), "1 line");
    assert_eq!(
        said,
        "not copied: 65537 bytes is over the 65536-byte clipboard limit"
    );
    assert_eq!(app.clip, None);
}

/// Inside tmux the copy is tmux's, whatever it says: the app writes no
/// escape of its own there, for tmux takes or drops one by `set-clipboard`.
#[test]
fn inside_tmux_a_copy_stages_nothing_for_the_terminal() {
    let mut app = app(Some("api"));
    let socket = std::path::PathBuf::from("/nonexistent-appclip/sock");
    app.server = Some(crate::inventory::ServerId::Selected(
        crate::meta::Selector::Socket(socket),
    ));
    app.pane = Some("%1".to_owned());
    let said = app.ship("alpha", "1 line");
    assert!(said.starts_with("copy failed: "), "{said}");
    assert_eq!(app.clip, None);
}

/// The fingerprints are built once for each lane: a frame, a key and a
/// click share one build, and another lane gets its own.
#[test]
fn the_turn_keys_are_built_once_for_each_lane() {
    let mut app = turned(&["one", "two"]);
    let first = app.keys();
    assert!(Rc::ptr_eq(&first, &app.keys()), "same lane, same build");
    app.lane = Rc::new(Lane {
        items: vec![said("one", 0)],
        coverage: Vec::new(),
    });
    assert_eq!(app.keys().len(), 1, "another lane, rebuilt");
    assert_eq!(first.len(), 2);
}

/// A press on a drawn turn marks it, a second press unmarks it, and a
/// press while writing returns to browsing, draft kept; the sidebar rule
/// still grabs beside it.
#[test]
fn a_press_on_a_turn_marks_it_and_a_second_unmarks_it() {
    let root = Root::new("turn-press");
    let (mut app, _reader) = housed(&root);
    let typed = writing(&mut app);
    if let Answer::View(mut read) = view("api", ID, 1, "") {
        read.lane.items = vec![said("one", 0), said("two", 1)];
        app.answer(Answer::View(read));
    }
    assert_eq!(app.compose(Key::Text(b"draft".to_vec()), typed), Some(()));
    let _ = framed(&mut app);
    let press = |column, row| super::Mouse {
        kind: super::MouseKind::Click,
        column,
        row,
    };
    let on_turn = |app: &App, column, row| {
        matches!(
            app.layout.hit(press(column, row)),
            Some(super::draw::Hit::Turn(_))
        )
    };
    let row = (0..45)
        .find(|row| on_turn(&app, 47, *row))
        .expect("a drawn turn");
    assert!(!on_turn(&app, 46, row), "the margin is no target");
    assert!(app.layout.grab(press(44, row)).is_some(), "the rule grabs");
    assert!(app.click(press(47, row)));
    assert!(!app.composing(), "a press browses");
    assert!(app.inputs.values().any(|input| input.draft() == "draft"));
    let marked = app.model.turn();
    assert!(marked.is_some_and(|key| app.keys().contains(&key)));
    assert!(app.click(press(47, row)));
    assert_eq!(app.model.turn(), None, "a second press unmarks it");
}

/// On a lane taller than the window a press finds the turn drawn on its
/// row, the gap row under a turn finds none, and the frame keeps each
/// turn above the newer ones plus that one gap row.
#[test]
fn a_press_finds_the_turn_drawn_on_the_row() {
    let bodies: Vec<String> = (0..30).map(|at| format!("word{at:02}")).collect();
    let mut app = turned(&bodies.iter().map(String::as_str).collect::<Vec<_>>());
    let drawn = framed(&mut app);
    let hit = |row: usize| {
        let (column, row) = (47, u16::try_from(row).expect("row"));
        match app.layout.hit(super::Mouse {
            kind: super::MouseKind::Click,
            column,
            row,
        }) {
            Some(super::draw::Hit::Turn(key)) => Some(key),
            _ => None,
        }
    };
    let body = |at: usize| {
        drawn
            .lines()
            .position(|l| l.contains(&format!("word{at:02}")))
    };
    let (first, last) = (0..30).fold((30, 0), |(lo, hi), at| match body(at) {
        Some(_) => (lo.min(at), hi.max(at)),
        None => (lo, hi),
    });
    assert!(first > 0 && last == 29, "the window is not the whole lane");
    for at in first..=last {
        let row = body(at).expect("a drawn body");
        assert_eq!(hit(row), Some(key_of(&app, at)), "the body row of {at}");
        assert_eq!(hit(row + 1), None, "the gap under {at}");
    }
    let spans: Vec<_> = app.layout.turns.iter().map(|t| (t.1, t.2)).collect();
    assert_eq!(spans[0].0, 0, "nothing beneath the newest turn");
    assert!(spans.windows(2).all(|w| w[1].0 == w[0].0 + w[0].1 + 1));
}

/// Issue 302: a press on the `↓ N newer turns · End` words scrolls the chat
/// to its newest turn, while writing too and with the draft kept; the
/// words are no turn, file or link target, and the row above them is a turn.
#[test]
fn a_press_on_the_newer_turns_hint_scrolls_to_the_newest_turn() {
    let root = Root::new("hint-press");
    let (mut app, _reader) = housed(&root);
    let typed = writing(&mut app);
    if let Answer::View(mut read) = view("api", ID, 1, "") {
        read.lane.items = (0..30)
            .map(|at| said(&format!("word{at:02}"), at))
            .collect();
        app.answer(Answer::View(read));
    }
    assert_eq!(app.compose(Key::Text(b"draft".to_vec()), typed), Some(()));
    let _ = framed(&mut app);
    for _ in 0..3 {
        app.model
            .wheel(true, app.layout.page_rows, app.layout.max_scroll);
    }
    let shown = framed(&mut app);
    assert!(!shown.contains("word29"), "scrolled off the newest turn");
    // Three notches hide nine rows: the three newest turns, three rows each.
    let (row, column) = shown
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            let byte = line.find("↓ 3 newer turns · End")?;
            Some((row, line[..byte].chars().count()))
        })
        .unwrap_or_else(|| panic!("the counted hint:\n{shown}"));
    let press = |column: usize, row: usize| super::Mouse {
        kind: super::MouseKind::Click,
        column: u16::try_from(column).expect("column"),
        row: u16::try_from(row).expect("row"),
    };
    let above = app.layout.hit(press(column + 2, row - 1));
    assert!(
        matches!(above, Some(super::draw::Hit::Turn(_))),
        "{above:?}"
    );
    let on = press(column + 2, row);
    assert!(app.layout.file(on).is_none() && !app.layout.link_at(on));
    assert!(matches!(app.layout.hit(on), Some(super::draw::Hit::Newest)));
    assert!(app.click(on), "a press on the words redraws");
    assert_eq!(app.model.scroll_rows(app.layout.page_rows), 0);
    assert_eq!(app.model.turn(), None, "no turn was marked");
    assert!(app.composing(), "still writing");
    assert_eq!(drafted(&app), "draft", "the draft is kept");
    let shown = framed(&mut app);
    assert!(shown.contains("word29"), "the newest turn is drawn");
    assert!(!shown.contains(" · End"), "no hint at the newest turn");
}

/// With none marked `[` / `]` take the newest turn the window shows (a
/// turn is shown when one of its rows is), and `[` brings an older turn
/// in by the least scroll.
#[test]
fn the_turn_keys_follow_the_rows_in_view() {
    let mut app = turned(&["a", "b", "c"]);
    let (old, mid, new) = (key_of(&app, 0), key_of(&app, 1), key_of(&app, 2));
    app.layout.page_rows = 5;
    app.layout.turns = vec![(new, 0, 3), (mid, 4, 3), (old, 8, 3)];
    let mark = |app: &mut App, scroll: usize| {
        app.model.set_turn(None);
        app.model.reveal(scroll, scroll + 5, 5);
        (app.step_turn(false), app.model.turn())
    };
    assert_eq!(
        mark(&mut app, 6),
        (true, Some(mid)),
        "newest is out of view"
    );
    assert_eq!(
        mark(&mut app, 3),
        (true, Some(mid)),
        "its top edge is the window's"
    );
    assert_eq!(mark(&mut app, 0), (true, Some(new)), "newest in view");
    assert!(app.step_turn(true));
    assert_eq!(app.model.scroll_rows(5), 2, "the least scroll shows `mid`");
    app.layout.turns = vec![(new, 5, 3)];
    assert_eq!(mark(&mut app, 0), (false, None), "wholly out of the window");
}

/// Another core at `2026.10.99`, which the app moves to `by`, or `why` not.
fn upgrade_by(by: Result<super::upgrade::Via, &str>) -> Answer {
    Answer::Core(Some(super::upgrade::Upgrade {
        version: "2026.10.99".to_owned(),
        restart: by.map_err(str::to_owned),
    }))
}

/// The chat window's respawn into `2026.10.99`.
fn respawn() -> super::upgrade::Via {
    super::upgrade::Via::from(crate::console::toggle::Restart {
        server: crate::inventory::ServerId::Ambient,
        pane: "%1".to_owned(),
        command: vec!["ae".to_owned(), "app".to_owned(), "api".to_owned()],
    })
}

/// A hand-started app's exec of the command link.
fn exec() -> super::upgrade::Via {
    super::upgrade::Via::Exec {
        link: "/home/me/.local/bin/ae".into(),
    }
}

/// Another core at `2026.10.99`, which the chat window restarts into, or
/// `why` not.
fn upgrade_to(restart: Result<(), &str>) -> Answer {
    upgrade_by(restart.map(|()| respawn()))
}

/// U3: an app with nothing to lose restarts into the linked core; writing
/// holds it back, and so does a draft kept after Esc, while the hint row
/// says it waits.
#[test]
fn a_restart_is_due_only_while_nothing_would_be_lost() {
    let root = Root::new("upgrade");
    let (mut app, _reader) = housed(&root);
    let t0 = Instant::now();
    assert_eq!(app.restart_due(t0), None, "no other core, no restart");
    app.answer(upgrade_to(Ok(())));
    let due = Some(("2026.10.99".to_owned(), respawn()));
    assert_eq!(app.restart_due(t0), due, "an idle app restarts");
    let waits = "ae 2026.10.99 installed - restarts when idle";
    assert_eq!(app.note().as_deref(), Some(waits));
    let at = writing(&mut app);
    assert_eq!(app.blocker(at), Some(Blocker::Composing));
    assert_eq!(app.compose(Key::Text(b"keep".to_vec()), at), Some(()));
    app.write(false);
    assert!(!app.composing());
    assert_eq!(
        app.blocker(at),
        Some(Blocker::Draft),
        "a kept draft is lost too"
    );
    assert_eq!(app.restart_due(at), None);
    assert_eq!(app.note().as_deref(), Some(waits));
}

/// G3: an exec that failed is never tried again for its version, the hint
/// row saying why; a link naming another version is tried.
#[test]
fn a_failed_exec_is_not_retried_for_its_version_and_says_why() {
    let mut app = app(None);
    app.answer(upgrade_by(Ok(exec())));
    let quiet = Instant::now() + super::REFRESH;
    let why = "Permission denied (os error 13)".to_owned();
    app.failed = Some(("2026.10.99".to_owned(), why));
    assert_eq!(app.restart_due(quiet), None);
    let said = "could not restart into ae 2026.10.99 - Permission denied (os error 13)";
    assert_eq!(app.note().as_deref(), Some(said));
    app.answer(Answer::Core(Some(super::upgrade::Upgrade {
        version: "2026.10.100".to_owned(),
        restart: Ok(exec()),
    })));
    let due = app.restart_due(quiet).map(|(version, _)| version);
    assert_eq!(due.as_deref(), Some("2026.10.100"), "another version");
    assert_eq!(
        app.note().as_deref(),
        Some("ae 2026.10.100 installed - restarts when idle")
    );
}

/// The operands the frozen spec leaves out: a dash is never a session, two
/// bare words are a usage error, and the internal view is not in the usage.
#[test]
fn the_app_takes_a_session_and_an_internal_view_last() {
    let words = |words: &[&str]| words.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
    for tail in [&["-h"][..], &["-w", "--app-view", "x"], &["web", "x"]] {
        assert!(super::operands(&words(tail)).is_none(), "{tail:?}");
    }
    assert!(!super::USAGE.contains("--app-view"), "internal");
    assert!(!super::USAGE.contains(super::OWN), "internal");
}

/// The own marker comes once, after the session and before the carrier;
/// anywhere else is a usage error, and no session name can be it.
#[test]
fn the_own_marker_is_taken_once_after_the_session_and_before_the_carrier() {
    let words = |words: &[&str]| words.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
    let read =
        |tail: &[&str]| super::operands(&words(tail)).map(|got| (got.session, got.own, got.handed));
    let (own, view) = (super::OWN, crate::app::handover::FLAG);
    let some = |session: Option<&str>, own, handed: Option<&str>| {
        Some((session.map(str::to_owned), own, handed.map(str::to_owned)))
    };
    assert_eq!(read(&[]), some(None, false, None));
    assert_eq!(read(&["web"]), some(Some("web"), false, None));
    assert_eq!(read(&[own]), some(None, true, None));
    assert_eq!(read(&["web", own]), some(Some("web"), true, None));
    assert_eq!(read(&[view, "x"]), some(None, false, Some("x")));
    assert_eq!(read(&[own, view, "x"]), some(None, true, Some("x")));
    assert_eq!(
        read(&["web", own, view, "x"]),
        some(Some("web"), true, Some("x"))
    );
    for tail in [
        &[own, own][..],
        &["web", own, own],
        &[own, "web"],
        &[view, "x", own],
        &["web", view, "x", own],
        &["web", "web", own],
        &["web", own, view],
    ] {
        assert!(read(tail).is_none(), "{tail:?}");
    }
    assert!(!crate::session_launch::name::is_session_name(own));
}

/// Only a start nothing marks as ae's own says the hint: not in a chat
/// window (a stamped pane), and not when tmux does not answer.
#[test]
fn only_a_start_in_no_chat_window_says_the_hint() {
    assert!(super::says_hint(false, None), "no tmux pane");
    assert!(super::says_hint(true, Some("")), "an unstamped pane");
    assert!(!super::says_hint(true, Some("uuid")), "a chat window");
    assert!(!super::says_hint(true, None), "tmux is silent: doubt");
}

/// Every argv ae builds for an app start of its own reads as one: the
/// landings' marker, an exec restart's carrier with a view or without.
#[test]
fn every_argv_ae_builds_for_its_own_start_reads_as_own() {
    let link = std::path::Path::new("/h/.local/bin/ae");
    let landing = [crate::own_app_argv(None), crate::own_app_argv(Some("web"))];
    let typed = vec!["web".to_owned()];
    let (newer, older) = ("2026.10.99", "2026.10.59");
    let exec = [
        super::upgrade::exec_argv(link, &typed, None, newer, older),
        super::upgrade::exec_argv(link, &[], None, newer, older),
        super::upgrade::exec_argv(link, &typed, Some("v1|x".to_owned()), newer, older),
    ];
    for argv in landing.iter().chain(&exec) {
        let at = argv.iter().position(|word| word == "app").expect("app");
        let got = super::operands(&argv[at + 1..]).expect("the grammar takes it");
        assert!(got.own || got.handed.is_some(), "{argv:?}");
    }
}

/// U3: every key read stamps the quiet window a restart waits out; an
/// armed key outranks the hint and holds it back; a hint-only upgrade,
/// or none, never restarts.
#[test]
fn a_key_holds_a_restart_and_a_hint_only_upgrade_never_restarts() {
    let mut app = app(None);
    app.fleet = one_row(None);
    app.model = Model::new(&app.fleet);
    app.answer(upgrade_to(Ok(())));
    let (mut keys, t0) = (crate::console::input::Keys::app(), Instant::now());
    let (_wake, wakes) = std::sync::mpsc::channel();
    let mut first = Some(loader::Wake::Keys(t0, b"x".to_vec()));
    assert!(super::drain(&mut app, &mut keys, &wakes, &mut first).is_some());
    assert_eq!(app.last_input, Some(t0));
    let quiet = t0 + super::REFRESH;
    let inside = quiet.checked_sub(Duration::from_millis(1)).expect("booted");
    assert_eq!(app.blocker(inside), Some(Blocker::RecentInput));
    assert!(
        app.restart_due(quiet).is_some(),
        "the quiet window has passed"
    );
    assert!(browse(&mut app, &Key::Text(b"q".to_vec()), quiet).is_some());
    assert_eq!(app.note().as_deref(), Some("q again to quit"));
    assert_eq!(app.blocker(quiet), Some(Blocker::Armed));
    app.armed = None;
    app.answer(upgrade_to(Err("auto_upgrade is off")));
    assert_eq!(app.restart_due(quiet), None);
    let only = "ae 2026.10.99 installed - restart ae app to use it";
    assert_eq!(app.note().as_deref(), Some(only));
    app.answer(Answer::Core(None));
    assert_eq!((app.note(), app.restart_due(quiet)), (None, None));
}

/// An `aedev` app that has drawn two seats, `colead` highlighted, with a
/// channel for what it asks the reader.
fn peeker() -> (App, std::sync::mpsc::Receiver<loader::Request>) {
    use crate::console::needs::SeatRef;
    let (tx, rx) = std::sync::mpsc::channel();
    let server = Some(crate::inventory::ServerId::Ambient);
    let mut app = App::new(Some("aedev".to_owned()), server, None, Some(tx));
    app.fleeted = true;
    app.world
        .sessions
        .push(SessionEntry::new("aedev", Status::Running));
    let mut fleet = one_row(Some("aedev"));
    fleet.rows[0].name = "aedev".to_owned();
    app.model = Model::new(&fleet);
    app.fleet = fleet;
    app.dirs
        .insert("aedev".to_owned(), PathBuf::from("/tmp/aedev"));
    let seat = |slot: &str, name: &str| {
        (
            name.to_owned(),
            Some(SeatRef {
                slot: slot.to_owned(),
                name: name.to_owned(),
            }),
        )
    };
    app.drawn = Some(super::Drawn {
        session: "aedev".to_owned(),
        uuid: ID.to_owned(),
        rows: vec![seat("main", "lead"), seat("worker.0", "colead")],
        focused: Some("colead".to_owned()),
    });
    (app, rx)
}

fn pressed(app: &mut App, bytes: &[u8]) -> Option<bool> {
    take_keys(app, vec![(Key::Text(bytes.to_vec()), Instant::now())])
}

/// `v` on a drawn, highlighted seat opens the panel and asks the reader
/// once for exactly that seat, pinned to a new generation each open; every
/// close tells the reader once.
#[test]
fn v_opens_the_panel_on_the_highlighted_seat_and_each_open_has_its_own_generation() {
    let (mut app, asks) = peeker();
    assert_eq!(pressed(&mut app, b"v"), Some(true));
    assert!(app.model.peek_open());
    let loader::Request::Peek(Some(ask)) = asks.try_recv().expect("an ask") else {
        panic!("the open asks for a peek");
    };
    assert_eq!(ask.generation, 1);
    assert_eq!((ask.session.as_str(), ask.uuid.as_str()), ("aedev", ID));
    assert_eq!(
        (ask.seat.slot.as_str(), ask.seat.name.as_str()),
        ("worker.0", "colead")
    );
    assert_eq!(ask.dir, PathBuf::from("/tmp/aedev"));
    assert!(asks.try_recv().is_err(), "one ask per open");
    assert_eq!(app.model.peek_generation(), Some(1));
    assert_eq!(
        app.peek.target,
        Some(("aedev".to_owned(), "colead".to_owned()))
    );
    assert_eq!(pressed(&mut app, b"q"), Some(true));
    assert!(!app.model.peek_open());
    assert!(matches!(asks.try_recv(), Ok(loader::Request::Peek(None))));
    assert!(asks.try_recv().is_err(), "one close per close");
    let _ = pressed(&mut app, b"v");
    let loader::Request::Peek(Some(again)) = asks.try_recv().expect("an ask") else {
        panic!("the open asks for a peek");
    };
    assert_eq!(again.generation, 2, "a reopen is a new generation");
}

/// A refusal before the panel opens says so on the hint row by seat and
/// reason, opens nothing and asks nothing.
#[test]
fn a_seat_the_frame_did_not_draw_refuses_by_name_and_asks_nothing() {
    let (mut app, asks) = peeker();
    app.drawn.as_mut().expect("drawn").focused = Some("ghost".to_owned());
    let _ = pressed(&mut app, b"v");
    assert!(!app.model.peek_open() && asks.try_recv().is_err());
    let (line, _) = app.flash.clone().expect("a flash");
    assert_eq!(
        line,
        "refused: peek ghost: no seat is shown to open in aedev"
    );
    app.drawn.as_mut().expect("drawn").focused = Some("colead".to_owned());
    app.drawn.as_mut().expect("drawn").session = "other".to_owned();
    let _ = pressed(&mut app, b"v");
    assert!(!app.model.peek_open() && asks.try_recv().is_err());
    let (line, _) = app.flash.clone().expect("a flash");
    assert_eq!(line, "refused: peek aedev: the view changed");
}

/// With no session selected `v` says so on the hint row and asks nothing.
#[test]
fn v_with_no_session_selected_says_so_and_asks_nothing() {
    let (mut app, asks) = peeker();
    app.model = Model::new(&Fleet::default());
    let _ = pressed(&mut app, b"v");
    assert!(!app.model.peek_open() && asks.try_recv().is_err());
    let said = "refused: peek: no session is selected";
    assert_eq!(
        app.flash.as_ref().map(|(text, _)| text.as_str()),
        Some(said)
    );
    assert!(framed(&mut app).contains(said), "the hint row says it");
}

/// A capture that lands after the panel closed, or for another open,
/// paints nothing and leaves no state behind.
#[test]
fn a_late_capture_for_a_closed_or_replaced_panel_is_dropped() {
    let (mut app, _asks) = peeker();
    let answer = |app: &mut App, generation, row: &str| {
        app.answer(Answer::Peek {
            generation,
            at: Instant::now(),
            body: super::Body::Lines(vec![row.to_owned()]),
        });
    };
    answer(&mut app, 1, "BEFORE OPEN");
    assert!(app.peek.shown.is_none() && app.peek.at.is_none());
    let _ = pressed(&mut app, b"v");
    answer(&mut app, 9, "WRONG GENERATION");
    assert!(app.peek.shown.is_none() && app.peek.at.is_none());
    answer(&mut app, 1, "RIGHT ONE");
    assert!(framed(&mut app).contains("RIGHT ONE"));
    let _ = pressed(&mut app, b"v");
    answer(&mut app, 1, "AFTER CLOSE");
    assert!(app.peek.shown.is_none() && app.peek.at.is_none());
    assert!(!framed(&mut app).contains("AFTER CLOSE"));
}

/// The open panel's capture ages on screen: one redraw for each new
/// second, none before the first answer or once closed.
#[test]
fn the_open_panel_redraws_once_for_each_new_second_of_age() {
    let (mut app, _asks) = peeker();
    let t0 = Instant::now();
    let after = |millis| t0 + Duration::from_millis(millis);
    let _ = pressed(&mut app, b"v");
    assert!(!app.lapse(after(5000)), "nothing captured yet");
    app.peek.at = Some(t0);
    app.peek.painted = Some(0);
    assert!(!app.lapse(after(999)), "still the second painted");
    assert!(app.lapse(after(1000)), "a new second");
    app.peek.painted = Some(1);
    assert!(!app.lapse(after(1999)));
    assert!(app.lapse(after(2000)));
    app.peek.at = Some(after(3000));
    assert_eq!(app.peek.age(t0), 0, "a capture from the future is 0 s old");
    app.peek.at = Some(t0);
    app.close_peek();
    app.peek.at = Some(t0);
    app.peek.painted = Some(0);
    assert!(!app.lapse(after(9000)), "a closed panel ages nothing");
}

/// With the panel open ^C quits, a paste and the mouse's presses die, and
/// a restart waits for the panel as it does for settings.
#[test]
fn the_open_panel_quits_on_ctrl_c_swallows_pastes_and_presses_and_holds_a_restart() {
    let (mut app, _asks) = peeker();
    let _ = pressed(&mut app, b"v");
    let now = Instant::now();
    assert_eq!(app.blocker(now), Some(Blocker::Settings));
    let pasted = vec![(Key::Pasted(b"hello".to_vec()), now)];
    assert_eq!(take_keys(&mut app, pasted), Some(false));
    assert!(app.model.peek_open());
    assert!(!app.composing(), "a paste starts no draft");
    for kind in [MouseKind::Click, MouseKind::Drag, MouseKind::Release] {
        let press = Mouse {
            kind,
            column: 20,
            row: 20,
        };
        assert!(!app.mouse(press), "{kind:?} dies");
        assert!(app.model.peek_open(), "{kind:?} leaves the panel");
    }
    assert_eq!(take_keys(&mut app, vec![(Key::Interrupt, now)]), None);
}

/// A close in the middle of a typed chunk hands the rest to the mode the
/// app is really in: `vq` closes, then `q` arms the quit; `vqq` quits.
#[test]
fn a_chunk_that_closes_the_panel_hands_its_rest_to_browse() {
    let (mut app, _asks) = peeker();
    let _ = pressed(&mut app, b"v");
    assert_eq!(pressed(&mut app, b"vq"), Some(true));
    assert!(!app.model.peek_open());
    assert_eq!(app.note().as_deref(), Some("q again to quit"));
    let _ = pressed(&mut app, b"v");
    assert_eq!(pressed(&mut app, b"vqq"), None, "the second q quits");
}

/// The pane's state belongs to the model's panel: the same generation
/// keeps it, another generation or a closed panel starts it afresh.
#[test]
fn the_pane_state_starts_afresh_when_the_models_panel_is_not_its_own() {
    let (mut app, _asks) = peeker();
    let holding = || super::PeekPane {
        generation: 1,
        target: Some(("aedev".to_owned(), "colead".to_owned())),
        shown: Some(super::Body::Lines(vec!["old".to_owned()])),
        held: Some(super::Body::Lines(vec!["newer".to_owned()])),
        at: Some(Instant::now()),
        painted: Some(3),
    };
    let afresh = |pane: &super::PeekPane, generation: u64| {
        pane.generation == generation
            && pane.target.is_none()
            && pane.shown.is_none()
            && pane.held.is_none()
            && pane.at.is_none()
            && pane.painted.is_none()
    };
    app.model.open_peek(1);
    app.peek = holding();
    app.peek_sync();
    assert!(!afresh(&app.peek, 1), "its own generation keeps its state");
    assert_eq!(app.peek.painted, Some(3));
    app.model.open_peek(2);
    app.peek_sync();
    assert!(afresh(&app.peek, 2), "another generation starts afresh");
    app.peek = holding();
    app.model.close_peek();
    app.peek_sync();
    assert!(afresh(&app.peek, 0), "a closed panel holds nothing");
}

/// The pane starts on the model's generation, every open its own, and
/// keeps its seat through the first frames: the composer row names the
/// viewed seat, before and after a capture.
#[test]
fn the_panel_starts_on_the_models_generation_and_its_title_names_the_seat() {
    let (mut app, _asks) = peeker();
    let _ = pressed(&mut app, b"v");
    assert_eq!(app.peek.generation, 1);
    let _ = pressed(&mut app, b"v");
    let _ = pressed(&mut app, b"v");
    assert_eq!(app.peek.generation, 2, "a reopen is its own generation");
    assert!(framed(&mut app).contains("viewing colead"));
    app.answer(Answer::Peek {
        generation: 2,
        at: Instant::now(),
        body: super::Body::Lines(vec!["ROW".to_owned()]),
    });
    let shown = framed(&mut app);
    assert!(shown.contains("viewing colead") && shown.contains("ROW"));
}

/// Two running sessions drawing `seats` (slot, name) each: the seat-click
/// binding fixture, api selected first. Each caller passes its own `tag`:
/// `Root` keys only pid plus tag, so shared tags race under `cargo test`.
fn bound_app(
    tag: &str,
    names: &[&str],
    seats: &[(&str, &str)],
) -> (Root, App, std::sync::mpsc::Receiver<loader::Request>) {
    use crate::console::needs::SeatRef;

    let root = Root::new(tag);
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App::new(
        Some(names[0].to_owned()),
        Some(crate::inventory::ServerId::Ambient),
        None,
        Some(tx),
    );
    (app.fleeted, app.loading) = (true, false);
    app.fleet = Fleet {
        home: Some(names[0].to_owned()),
        rows: Vec::new(),
    };
    for (at, name) in names.iter().enumerate() {
        let dir = session(&root, name, "");
        app.dirs.insert((*name).to_owned(), dir);
        app.ids.insert((*name).to_owned(), ID.to_owned());
        app.world.sessions.push(entry(name, Status::Running, None));
        app.fleet.rows.push(row(name, at + 1, at == 0));
        let roster: Vec<SeatRef> = seats
            .iter()
            .map(|(slot, name)| SeatRef {
                slot: (*slot).into(),
                name: (*name).into(),
            })
            .collect();
        app.facts.insert(
            (*name).to_owned(),
            fleet::Facts::Seats {
                id: ID.to_owned(),
                agents: roster
                    .iter()
                    .map(|seat| tmux::PickerAgent {
                        name: seat.name.clone(),
                        profile: "sol61x".into(),
                        state: "working".into(),
                        pane: "%7".into(),
                        client: String::new(),
                        model: String::new(),
                        effort: String::new(),
                        drift: false,
                        task: false,
                    })
                    .collect(),
            },
        );
        app.shown.insert(
            (*name).to_owned(),
            super::Shown {
                id: ID.to_owned(),
                seq: 1,
                lane: Rc::new(Lane {
                    coverage: Vec::new(),
                    items: Vec::new(),
                }),
                roster: Some(roster),
            },
        );
    }
    app.model = Model::new(&app.fleet);
    app.model.set_focus(Some("colead".to_owned()));
    app.show();
    (root, app, rx)
}

/// The click on the drawn seat target `seat` in `shown`.
fn seat_cell(shown: &str, app: &App, seat: &str) -> super::Mouse {
    let click = |column: usize, row: usize| super::Mouse {
        kind: super::MouseKind::Click,
        column: u16::try_from(column).expect("frame column"),
        row: u16::try_from(row).expect("frame row"),
    };
    shown
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            line.match_indices(seat).find_map(|(byte, _)| {
                let column = line[..byte].chars().count();
                matches!(
                    app.layout.hit(click(column, row)),
                    Some(super::draw::Hit::Seat(name)) if name == seat
                )
                .then_some(click(column, row))
            })
        })
        .unwrap_or_else(|| panic!("drawn seat target {seat}"))
}

/// B2: a seat click delayed past the frame that drew it refuses by the
/// clicked name once that seat is gone, and never falls back to the
/// highlight it would have opened instead.
#[test]
fn a_delayed_seat_click_after_removal_refuses_by_name_and_opens_nothing() {
    let (_root, mut app, asks) = bound_app(
        "livepeek-bind-removal",
        &["api"],
        &[("main", "lead"), ("worker.0", "colead")],
    );
    let first = framed(&mut app);
    let at = seat_cell(&first, &app, "colead");
    let origin = Instant::now();
    if let Some(fleet::Facts::Seats { agents, .. }) = app.facts.get_mut("api") {
        agents.retain(|agent| agent.name != "colead");
    }
    if let Some(shown) = app.shown.get_mut("api")
        && let Some(roster) = shown.roster.as_mut()
    {
        roster.retain(|seat| seat.name != "colead");
    }
    let _second = framed(&mut app);
    assert_eq!(
        app.drawn.as_ref().and_then(|drawn| drawn.focused.clone()),
        Some("lead".to_owned()),
        "the trap: a live highlight to fall back to"
    );
    assert!(app.mouse_made(at, origin), "the stale click is taken");
    assert!(!app.model.peek_open(), "nothing opens");
    assert!(asks.try_recv().is_err(), "nothing asked");
    let (line, _) = app.flash.clone().expect("a refusal");
    assert_eq!(line, "refused: peek colead: the view changed");
}

/// B2: the same delayed click after a session switch refuses too, even
/// when the new session seats the same name: the click stays bound to
/// the frame that drew it.
#[test]
fn a_delayed_seat_click_after_a_session_switch_refuses_despite_same_names() {
    let (_root, mut app, asks) = bound_app(
        "livepeek-bind-switch",
        &["api", "web"],
        &[("main", "lead"), ("worker.0", "colead")],
    );
    let first = framed(&mut app);
    let at = seat_cell(&first, &app, "colead");
    let origin = Instant::now();
    let _ = app.model.key(Browse::Digit(2), &app.fleet, false);
    app.show();
    let _second = framed(&mut app);
    assert_eq!(app.model.selected(), Some("web"));
    assert!(
        app.drawn
            .as_ref()
            .is_some_and(|drawn| drawn.rows.iter().any(|(name, _)| name == "colead")),
        "the trap: the new session seats the same name"
    );
    assert!(app.mouse_made(at, origin), "the stale click is taken");
    assert!(!app.model.peek_open(), "nothing opens");
    assert!(asks.try_recv().is_err(), "nothing asked");
    let (line, _) = app.flash.clone().expect("a refusal");
    assert_eq!(line, "refused: peek colead: the view changed");
}

/// B2: while a seat is viewed the wheel scrolls the viewed screen where
/// the chat column is drawn, and dies anywhere else — the keys row
/// scrolls nothing.
#[test]
fn a_wheel_while_viewing_scrolls_only_where_it_lands() {
    let (mut app, _asks) = peeker();
    let _ = pressed(&mut app, b"v");
    assert!(app.model.peek_open());
    let _shown = framed(&mut app);
    let wheel = |column: u16, row: u16| super::Mouse {
        kind: super::MouseKind::WheelUp,
        column,
        row,
    };
    let chat = (0..160)
        .find(|column| app.layout.in_chat(wheel(*column, 10)))
        .expect("a chat cell");
    let before = app.model.peek_back().unwrap_or(0);
    assert!(app.mouse(wheel(chat, 10)), "the viewed screen scrolls");
    assert!(
        app.model.peek_back().unwrap_or(0) > before,
        "the notch lands on the view"
    );
    assert!(!app.mouse(wheel(100, 44)), "the keys row is no scroll");
    assert_eq!(
        app.model.peek_back().unwrap_or(0),
        before.saturating_add(crate::app::model::WHEEL_ROWS),
        "nothing else moved it"
    );
}

/// B2: a wheel over the viewed screen is handled at once even where the
/// hidden chat's bound is unproven: the view waits for no frame.
#[test]
fn a_viewed_wheel_is_never_deferred_on_the_hidden_chat_bound() {
    let (mut app, _asks) = peeker();
    let _ = pressed(&mut app, b"v");
    assert!(app.model.peek_open());
    let _shown = framed(&mut app);
    let chat = (0..160)
        .find(|column| {
            app.layout.in_chat(super::Mouse {
                kind: super::MouseKind::WheelUp,
                column: *column,
                row: 10,
            })
        })
        .expect("a chat cell");
    let wheel = Key::Mouse(super::Mouse {
        kind: super::MouseKind::WheelUp,
        column: chat,
        row: 10,
    });
    app.layout.complete = false;
    app.layout.max_scroll = 0;
    assert!(
        !app.unbounded(&wheel),
        "the view owns its wheel despite the hidden bound"
    );
    let _ = take_keys(&mut app, vec![(wheel.clone(), Instant::now())]);
    assert!(app.deferred.is_empty(), "nothing deferred");
    let _ = pressed(&mut app, b"q");
    assert!(!app.model.peek_open());
    app.layout.complete = false;
    app.layout.max_scroll = 0;
    assert!(
        app.unbounded(&wheel),
        "the same bound defers once the chat is back"
    );
}

/// A capture that aged and any other line that ended lapse in one call,
/// each saying it did.
#[test]
fn an_aged_capture_lapses_together_with_every_line_that_ends() {
    let t0 = Instant::now();
    let at = t0 + super::FLASH_WINDOW + Duration::from_millis(1);
    for (armed, flash, word) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
        (true, true, true),
    ] {
        let (mut app, _asks) = peeker();
        let _ = pressed(&mut app, b"v");
        app.peek.at = Some(t0);
        app.peek.painted = Some(0);
        app.armed = armed.then_some(super::Armed {
            at: t0,
            what: super::Arm::Quit,
        });
        app.flash = flash.then(|| ("paste not taken: x".to_owned(), t0));
        app.settling = word.then(|| super::Settling {
            at: t0,
            id: "r".to_owned(),
            word: "delivered".to_owned(),
            about: "aedev".to_owned(),
        });
        assert!(app.lapse(at), "{armed} {flash} {word} lapsed");
        let left = (
            app.armed.is_some(),
            app.flash.is_some(),
            app.settling.is_some(),
        );
        assert_eq!(left, (false, false, false), "{armed} {flash} {word}");
    }
}

/// A chunk that opens or closes the panel redraws whatever bytes follow it,
/// also ones the next mode swallows.
#[test]
fn a_chunk_that_opens_or_closes_the_panel_redraws_whatever_follows() {
    let (mut app, _asks) = peeker();
    assert_eq!(pressed(&mut app, b"vz"), Some(true), "opens; z dies");
    assert!(app.model.peek_open());
    assert_eq!(
        pressed(&mut app, b"vz"),
        Some(true),
        "closes; z is browse's"
    );
    assert!(!app.model.peek_open());
}

/// With the panel open a scroll key redraws and a key it swallows does not.
#[test]
fn an_open_panel_redraws_for_a_scroll_and_not_for_a_swallowed_key() {
    let (mut app, _asks) = peeker();
    let _ = pressed(&mut app, b"v");
    assert_eq!(pressed(&mut app, b"k"), Some(true), "up scrolls");
    assert_eq!(app.model.peek_back(), Some(1));
    assert_eq!(pressed(&mut app, b"z"), Some(false), "z is swallowed");
    assert_eq!(app.model.peek_back(), Some(1));
}

/// The frame `app` draws, and the cell where `word` starts in it.
fn shown(app: &mut App, word: &str) -> (Buffer, u16, u16) {
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
    app.frame(&mut buf);
    let spelled: Vec<String> = word.chars().map(String::from).collect();
    let (width, height) = (buf.area.width, buf.area.height);
    let found = (0..height).find_map(|y| {
        (0..width).find_map(|x| {
            let fits = (0..)
                .zip(&spelled)
                .all(|(at, ch)| x + at < width && buf[(x + at, y)].symbol() == ch.as_str());
            fits.then_some((x, y))
        })
    });
    let (x, y) = found.expect("a drawn word");
    (buf, x, y)
}

fn lit(buf: &Buffer, y: u16) -> Vec<u16> {
    let inverse = |x: &u16| {
        let cell = &buf[(*x, y)];
        cell.modifier
            .contains(ratatui_core::style::Modifier::REVERSED)
    };
    (0..buf.area.width).filter(inverse).collect()
}

fn at(kind: MouseKind, column: u16, row: u16) -> Mouse {
    Mouse { kind, column, row }
}

fn staged(app: &App) -> Option<&str> {
    match app.select.as_ref().map(|held| &held.stage) {
        Some(super::Stage::Ready(text)) => Some(text),
        _ => None,
    }
}

/// A read landing in the middle of a drag waits: the release copies the
/// cells the human saw, a wide and a combined symbol whole, and then the
/// read lands.
#[test]
fn a_read_during_a_drag_waits_for_the_copy_it_would_have_cleared() {
    let mut app = turned(&["qq\u{4f60}e\u{301}zz beta"]);
    let (_, x, y) = shown(&mut app, "qq");
    app.mouse(at(MouseKind::Click, x + 2, y));
    app.mouse(at(MouseKind::Drag, x + 4, y));
    app.answer(view("api", ID, 1, "omega"));
    assert!(app.shown.is_empty(), "the read waits for the drag");
    app.mouse(at(MouseKind::Release, x + 4, y));
    let _ = shown(&mut app, "qq");
    assert_eq!(staged(&app), Some("\u{4f60}e\u{301}"), "the cells seen");
    assert!(app.owed());
    assert_eq!(app.clip.as_deref(), Some("\u{4f60}e\u{301}"));
    let landed = app.shown.get("api").map(|shown| shown.seq);
    assert_eq!(landed, Some(1), "then the read lands");
}

/// Whatever ends a drag lets the waiting read land, and a drag that goes
/// quiet for the hold clears itself, saying so: nothing waits for good.
#[test]
fn every_end_of_a_drag_lets_the_waiting_read_land() {
    for end in 0..4 {
        let mut app = turned(&["alpha beta gamma"]);
        let (_, x, y) = shown(&mut app, "alpha");
        app.mouse(at(MouseKind::Click, x, y));
        app.mouse(at(MouseKind::Drag, x + 3, y));
        app.answer(view("api", ID, 1, "omega"));
        assert!(app.shown.is_empty(), "end {end}: the read waits");
        match end {
            0 => drop(take_keys(&mut app, vec![(Key::Escape, Instant::now())])),
            1 => drop(app.mouse(at(MouseKind::Click, 100, 44))),
            2 => {
                app.model.open_settings(1);
                app.model.close_settings();
                assert!(app.stale_selection());
            }
            _ => {
                let quiet = Instant::now().checked_sub(super::LANE_HOLD * 2);
                app.select.as_mut().expect("a drag").touched = quiet.expect("uptime");
            }
        }
        let _ = app.owed();
        assert_eq!(app.shown.len(), 1, "end {end}: the read landed");
        assert!(app.select.is_none() && app.parked.is_empty(), "end {end}");
        let said = app.flash.as_ref().map(|(line, _)| line.as_str());
        assert_eq!(
            said.is_some_and(|l| l.starts_with("selection cleared")),
            end == 3
        );
    }
}

/// A held drag keeps the newest read of each kind and name, in arrival
/// order: a newer one replaces the older, another name or kind stays, and
/// a view carries what the one it replaced alone read - of the same id.
#[test]
fn parked_reads_keep_the_newest_of_each_kind_and_name() {
    let mut app = turned(&["alpha beta gamma"]);
    let (_, x, y) = shown(&mut app, "alpha");
    app.mouse(at(MouseKind::Click, x, y));
    app.mouse(at(MouseKind::Drag, x + 3, y));
    let look = |name: &str| Answer::Look {
        name: name.to_owned(),
        look: None,
        zone: None,
    };
    let seen = |name: &str, id: &str, seq, needs: bool| {
        let Answer::View(mut read) = view(name, id, seq, "omega") else {
            unreachable!("a view");
        };
        read.needs = needs.then(|| crate::console::needs::Section { rows: Vec::new() });
        Answer::View(read)
    };
    let answers = vec![
        Answer::Fleet(read_of(&app, &[])),
        look("b"),
        look("a"),
        look("a"),
        seen("c", ID, 1, true),
        seen("c", ID, 2, false),
        seen("d", ID, 1, true),
        seen("d", "other", 2, false),
        seen("e", ID, 1, false),
        Answer::Fleet(read_of(&app, &[])),
    ];
    for answer in answers {
        app.answer(answer);
    }
    let kinds: Vec<String> = app
        .parked
        .iter()
        .map(|answer| match answer {
            Answer::Fleet(_) => "fleet".to_owned(),
            Answer::Look { name, .. } => format!("look {name}"),
            Answer::View(read) => {
                format!("{} {} {}", read.name, read.seq, read.needs.is_some())
            }
            _ => "other".to_owned(),
        })
        .collect();
    let want = [
        "look b",
        "look a",
        "c 2 true",
        "d 2 false",
        "e 1 false",
        "fleet",
    ];
    assert_eq!(kinds, want);
}

/// Parked reads land after a shipped copy without touching its highlight or
/// saying the selection was cleared; a drag that went quiet after its
/// release, its copy still unframed, is cleared and says so.
#[test]
fn parked_reads_land_after_a_copy_and_a_quiet_requested_drag_is_cleared() {
    let mut app = turned(&["alpha beta gamma"]);
    let (_, x, y) = shown(&mut app, "alpha");
    app.mouse(at(MouseKind::Click, x, y));
    app.mouse(at(MouseKind::Drag, x + 3, y));
    app.answer(view("api", ID, 1, "omega"));
    app.mouse(at(MouseKind::Release, x + 3, y));
    let _ = shown(&mut app, "alpha");
    let _ = app.owed();
    let said = app.flash.as_ref().map(|(line, _)| line.as_str());
    assert!(said.is_some_and(|line| line.starts_with("sent 1 line")));
    assert!(app.select.is_some() && app.parked.is_empty() && app.shown.len() == 1);
    app.mouse(at(MouseKind::Click, x, y));
    app.mouse(at(MouseKind::Drag, x + 3, y));
    app.mouse(at(MouseKind::Release, x + 3, y));
    app.answer(view("api", ID, 2, "omega"));
    let quiet = Instant::now().checked_sub(super::LANE_HOLD * 2);
    app.select.as_mut().expect("a drag").touched = quiet.expect("uptime");
    let _ = app.owed();
    assert!(app.select.is_none() && app.parked.is_empty());
    let said = app.flash.as_ref().map(|(line, _)| line.as_str());
    assert!(said.is_some_and(|line| line.starts_with("selection cleared: no mouse")));
}

/// One frame can owe a queued ask's delivery and the parked reads it lets
/// in together: each makes the frame due, whatever the other did.
#[test]
fn a_frame_owes_the_asks_and_the_parked_reads_it_lets_in_together() {
    let mut app = turned(&["alpha beta gamma"]);
    let (_, x, y) = shown(&mut app, "alpha");
    app.mouse(at(MouseKind::Click, x, y));
    app.mouse(at(MouseKind::Drag, x + 3, y));
    app.answer(view("api", ID, 1, "omega"));
    let quiet = Instant::now().checked_sub(super::LANE_HOLD * 2);
    app.select.as_mut().expect("a drag").touched = quiet.expect("uptime");
    app.queued.push(super::Queued {
        key: ("api".to_owned(), "id".to_owned()),
        entry: 0,
        raw: b"hi".to_vec(),
        seat: "lead".to_owned(),
        body: "hi".to_owned(),
        about: "api".to_owned(),
        note: None,
    });
    assert!(app.owed(), "an ask ran and a read landed");
    assert!(app.queued.is_empty() && app.parked.is_empty());
}

/// A held drag is never restarted away, however long the keys were quiet:
/// the restart waits for the release and its copy.
#[test]
fn no_restart_while_a_drag_is_held() {
    let mut app = turned(&["alpha beta gamma"]);
    app.answer(upgrade_to(Ok(())));
    let (_, x, y) = shown(&mut app, "alpha");
    app.mouse(at(MouseKind::Click, x, y));
    app.mouse(at(MouseKind::Drag, x + 3, y));
    let quiet = Instant::now() + super::REFRESH * 2;
    assert_eq!(app.restart_due(quiet), None, "the drag is held");
    app.mouse(at(MouseKind::Release, x + 3, y));
    let _ = shown(&mut app, "alpha");
    let _ = app.owed();
    app.clip = None;
    assert!(app.restart_due(quiet).is_some(), "released and copied");
}

/// The open asks' ages are judged at the press while the button is down:
/// a second turning under a drag does not re-read them (the header they
/// show in is no selection, so the read itself is what is pinned).
#[test]
fn the_open_asks_ages_wait_for_a_held_drag() {
    let mut app = turned(&["alpha beta gamma"]);
    let entry = crate::digest::SessionEntry::new("api", crate::digest::Status::Running);
    app.world.sessions = vec![entry];
    app.dirs
        .insert("api".to_owned(), PathBuf::from("/nonexistent"));
    let since = (crate::time::Timestamp::now().epoch() - 30) * 1_000_000;
    app.needs.insert(
        "api".to_owned(),
        crate::console::needs::Section {
            rows: vec![crate::console::needs::Row {
                seat: seat_ref("main", "lead"),
                lead_pair: true,
                verdict: crate::console::needs::Verdict::Reason(Reason::WaitingUser),
                source: crate::console::needs::Source::Declaration,
                stale: None,
                since_micros: Some(since),
                detail: "BODYWORD".to_owned(),
                record: None,
            }],
        },
    );
    app.show();
    let aged = |app: &App| app.overview.open.first().and_then(|ask| ask.age_secs);
    let at_press = aged(&app);
    assert!(at_press.is_some(), "an open ask is read");
    app.lane = Rc::new(Lane {
        items: vec![said("alpha beta gamma", 0)],
        coverage: Vec::new(),
    });
    let (_, x, y) = shown(&mut app, "alpha");
    app.mouse(at(MouseKind::Click, x, y));
    app.mouse(at(MouseKind::Drag, x + 3, y));
    std::thread::sleep(Duration::from_millis(1100));
    let _ = app.owed();
    assert_eq!(aged(&app), at_press, "the age waits for the held drag");
    app.mouse(at(MouseKind::Release, x + 3, y));
    let _ = shown(&mut app, "alpha");
    assert_eq!(staged(&app), Some("alph"), "the cells seen at the press");
}

/// The drag paints the cells it covers, the release asks for a copy of
/// the cells a frame painted - the release cell included, though no drag
/// event reached it - and the copy leaves only after that frame.
#[test]
fn a_drag_paints_its_cells_and_the_release_ships_what_the_frame_painted() {
    let mut app = turned(&["alpha beta gamma", "second"]);
    let (_, x, y) = shown(&mut app, "alpha beta gamma");
    assert!(app.mouse(at(MouseKind::Click, x + 6, y)));
    let held = app.select.as_ref().expect("a press arms");
    assert!(held.down && !held.sel.moved());
    assert!(app.model.turn().is_some(), "the press marked the turn");
    assert!(app.mouse(at(MouseKind::Drag, x + 8, y)));
    assert_eq!(app.model.turn(), None, "motion puts the mark back");
    assert!(app.mouse(at(MouseKind::Release, x + 9, y)));
    assert_eq!(staged(&app), None, "nothing leaves before a frame");
    let (buf, ..) = shown(&mut app, "alpha beta gamma");
    assert_eq!(lit(&buf, y), (x + 6..=x + 9).collect::<Vec<_>>());
    assert_eq!(staged(&app), Some("beta"), "the very cells painted");
    assert!(app.ship_staged());
    let said = app.flash.as_ref().map(|(line, _)| line.as_str());
    assert!(said.is_some_and(|line| line.starts_with("sent 1 line to the terminal")));
    assert!(!app.ship_staged(), "a copy ships once");
    let (buf, ..) = shown(&mut app, "alpha");
    assert_eq!(lit(&buf, y).len(), 4, "the highlight stays");
}

/// A press and release with no motion is the click it always was: the
/// turn stays marked, nothing is staged, nothing is highlighted.
#[test]
fn a_plain_click_selects_nothing_and_keeps_its_old_behaviour() {
    let mut app = turned(&["alpha beta gamma"]);
    let (_, x, y) = shown(&mut app, "alpha");
    assert!(app.mouse(at(MouseKind::Click, x + 2, y)));
    let marked = app.model.turn();
    assert!(marked.is_some());
    let _ = app.mouse(at(MouseKind::Release, x + 2, y));
    assert!(app.select.is_none() && !app.ship_staged());
    assert_eq!(app.model.turn(), marked, "the mark stays");
    let (buf, ..) = shown(&mut app, "alpha");
    assert!(lit(&buf, y).is_empty());
}

/// A press anywhere ends the selection, and a copy staged but not yet
/// shipped goes with it; so does Esc, which is consumed.
#[test]
fn a_press_or_esc_ends_a_selection_and_its_unshipped_copy() {
    for esc in [false, true] {
        let mut app = turned(&["alpha beta gamma"]);
        let (_, x, y) = shown(&mut app, "alpha");
        app.mouse(at(MouseKind::Click, x, y));
        app.mouse(at(MouseKind::Drag, x + 3, y));
        app.mouse(at(MouseKind::Release, x + 3, y));
        if esc {
            let keys = vec![(Key::Escape, Instant::now())];
            assert_eq!(take_keys(&mut app, keys), Some(true));
            assert!(app.model.selected().is_some(), "Esc only cleared it");
        } else {
            app.mouse(at(MouseKind::Click, 100, 44));
        }
        assert!(app.select.is_none());
        let (buf, ..) = shown(&mut app, "alpha");
        assert!(lit(&buf, y).is_empty() && !app.ship_staged(), "esc={esc}");
    }
}

/// A panel change drops the selection even when the panel changes back
/// to the very cells it held; a changed cell drops it too, saying so.
#[test]
fn a_selection_dies_with_its_panel_and_with_its_cells() {
    let mut app = turned(&["alpha beta gamma"]);
    let (_, x, y) = shown(&mut app, "alpha");
    let drag = |app: &mut App| {
        app.mouse(at(MouseKind::Click, x, y));
        app.mouse(at(MouseKind::Drag, x + 3, y));
    };
    drag(&mut app);
    app.model.open_settings(1);
    app.model.close_settings();
    assert!(app.stale_selection(), "away and back is still a change");
    assert!(app.select.is_none());
    drag(&mut app);
    app.mouse(at(MouseKind::Release, x + 3, y));
    app.lane = Rc::new(Lane {
        items: vec![said("omega beta gamma", 0)],
        coverage: Vec::new(),
    });
    let (buf, ..) = shown(&mut app, "omega");
    assert!(app.select.is_none() && lit(&buf, y).is_empty());
    let said = app.flash.as_ref().map(|(line, _)| line.as_str());
    assert_eq!(said, Some("selection cleared: the screen changed"));
    assert!(!app.ship_staged() && std::mem::take(&mut app.again));
}

/// A press on a frame a later one replaced arms nothing and still ends
/// a selection held; a border grab arms nothing either.
#[test]
fn a_stale_press_and_a_border_grab_arm_no_selection() {
    let mut app = turned(&["alpha beta gamma"]);
    let (_, x, y) = shown(&mut app, "alpha");
    app.mouse(at(MouseKind::Click, x, y));
    app.mouse(at(MouseKind::Drag, x + 3, y));
    assert!(app.mouse_in(at(MouseKind::Click, x, y), false));
    assert!(app.select.is_none(), "it ended the old, armed no new one");
    let rule = (0..160).find(|c| app.layout.grab(at(MouseKind::Click, *c, y)).is_some());
    let rule = rule.expect("a sidebar rule on the row");
    app.mouse(at(MouseKind::Click, rule, y));
    assert!(app.select.is_none(), "the grab wins");
}

/// A press that takes a lit selection away asks for the redraw that
/// removes the highlight, on the frame on screen and on one a later frame
/// replaced, though the press itself acts on nothing.
#[test]
fn a_press_that_ends_a_highlight_asks_for_the_redraw_on_either_frame() {
    for historical in [false, true] {
        let mut app = turned(&["alpha beta gamma"]);
        let origin = Instant::now();
        let (_, x, y) = shown(&mut app, "alpha");
        let press = |app: &mut App| {
            let inert = at(MouseKind::Click, 100, 44);
            if historical {
                app.mouse_made(inert, origin)
            } else {
                app.mouse(inert)
            }
        };
        assert!(
            !press(&mut app),
            "{historical}: on its own it acts on nothing"
        );
        app.mouse(at(MouseKind::Click, x, y));
        app.mouse(at(MouseKind::Drag, x + 3, y));
        let (buf, ..) = shown(&mut app, "alpha");
        let held = app.select.as_ref().expect("a drag holds a selection");
        assert!(held.sel.moved() && !lit(&buf, y).is_empty(), "{historical}");
        assert!(
            press(&mut app),
            "{historical}: the highlight must be redrawn away"
        );
        assert!(app.select.is_none(), "{historical}: the press ended it");
        let (buf, ..) = shown(&mut app, "alpha");
        assert!(lit(&buf, y).is_empty(), "{historical}: nothing stays lit");
    }
}

/// Every motion that moves the lit end asks for a frame, not only the
/// first one: the highlight follows the pointer.
#[test]
fn each_drag_that_moves_the_lit_end_asks_for_a_frame() {
    let mut app = turned(&["alpha beta gamma"]);
    let (_, x, y) = shown(&mut app, "alpha");
    app.mouse(at(MouseKind::Click, x, y));
    assert!(app.mouse(at(MouseKind::Drag, x + 1, y)), "the first motion");
    let _ = shown(&mut app, "alpha");
    assert!(app.mouse(at(MouseKind::Drag, x + 3, y)), "a later motion");
    let (buf, ..) = shown(&mut app, "alpha");
    assert_eq!(lit(&buf, y), (x..=x + 3).collect::<Vec<_>>());
}

/// A frame is owed after a shipped copy and after a hint with no room,
/// and only then; a waiting hint is taken.
#[test]
fn a_frame_is_owed_for_a_shipped_copy_or_a_waiting_hint() {
    for (copy, hint) in [(false, false), (true, false), (false, true)] {
        let mut app = turned(&["alpha beta gamma"]);
        let (_, x, y) = shown(&mut app, "alpha");
        if copy || hint {
            app.mouse(at(MouseKind::Click, x, y));
            app.mouse(at(MouseKind::Drag, x + 3, y));
            app.mouse(at(MouseKind::Release, x + 3, y));
        }
        if hint {
            // The screen changes before the copy leaves: dropped, with a hint.
            app.lane = Rc::new(Lane {
                items: vec![said("omega beta gamma", 0)],
                coverage: Vec::new(),
            });
            let _ = shown(&mut app, "omega");
        } else if copy {
            let _ = shown(&mut app, "alpha");
        }
        assert_eq!(app.again, hint, "copy {copy}, hint {hint}: a hint waits");
        assert_eq!(app.shipped(), copy || hint, "copy {copy}, hint {hint}");
        assert!(!app.again, "copy {copy}, hint {hint}: the hint is taken");
    }
}

/// The loop settles the copy and the queued asks together: a frame is due
/// when either ran, and each runs whatever the other did.
#[test]
fn a_frame_settles_the_copy_and_the_queued_asks_together() {
    for (copy, ask) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut app = turned(&["alpha beta gamma"]);
        let (_, x, y) = shown(&mut app, "alpha");
        if copy {
            app.mouse(at(MouseKind::Click, x, y));
            app.mouse(at(MouseKind::Drag, x + 3, y));
            app.mouse(at(MouseKind::Release, x + 3, y));
            let _ = shown(&mut app, "alpha");
        }
        if ask {
            app.queued.push(super::Queued {
                key: ("api".to_owned(), "id".to_owned()),
                entry: 0,
                raw: b"hi".to_vec(),
                seat: "lead".to_owned(),
                body: "hi".to_owned(),
                about: "api".to_owned(),
                note: None,
            });
        }
        assert_eq!(app.owed(), copy || ask, "copy {copy}, ask {ask}");
        assert!(app.queued.is_empty(), "copy {copy}, ask {ask}: asks ran");
        assert_eq!(app.flash.is_some(), copy, "copy {copy}, ask {ask}: shipped");
    }
}

/// A selection that lost nothing a person saw or asked for - a press that
/// never moved, or a highlight whose copy already left - goes without a
/// word when the screen changes under it.
#[test]
fn a_selection_that_lost_nothing_goes_quietly_when_the_screen_changes() {
    for copied in [false, true] {
        let mut app = turned(&["alpha beta gamma"]);
        let (_, x, y) = shown(&mut app, "alpha");
        app.mouse(at(MouseKind::Click, x, y));
        if copied {
            app.mouse(at(MouseKind::Drag, x + 3, y));
            app.mouse(at(MouseKind::Release, x + 3, y));
            let _ = shown(&mut app, "alpha");
            assert!(app.ship_staged(), "the copy left");
        }
        let before = app.flash.as_ref().map(|(line, _)| line.clone());
        app.lane = Rc::new(Lane {
            items: vec![said("omega beta gamma", 0)],
            coverage: Vec::new(),
        });
        let (buf, ..) = shown(&mut app, "omega");
        assert!(app.select.is_none() && lit(&buf, y).is_empty(), "{copied}");
        let after = app.flash.as_ref().map(|(line, _)| line.clone());
        assert_eq!(after, before, "{copied}: nothing new is said");
        assert!(!std::mem::take(&mut app.again), "{copied}: no hint owed");
    }
}

/// A key that finds a lit selection left behind by a panel change asks for
/// the redraw that removes the highlight, though the key itself does
/// nothing.
#[test]
fn a_key_after_a_panel_change_redraws_the_stale_highlight_away() {
    for stale in [false, true] {
        let mut app = turned(&["alpha beta gamma"]);
        let (_, x, y) = shown(&mut app, "alpha");
        if stale {
            app.mouse(at(MouseKind::Click, x, y));
            app.mouse(at(MouseKind::Drag, x + 3, y));
            let _ = shown(&mut app, "alpha");
            app.model.open_settings(1);
            app.model.close_settings();
        }
        let keys = vec![(Key::Text(b"~".to_vec()), Instant::now())];
        assert_eq!(take_keys(&mut app, keys), Some(stale), "stale {stale}");
        assert!(app.select.is_none(), "stale {stale}");
    }
}

/// In a lane taller than the screen the marked turn's bar is blanked on
/// the very cells it is drawn on, whichever turns scrolled out above.
#[test]
fn a_marked_turns_bar_is_blanked_where_it_is_drawn_in_a_long_lane() {
    let bodies: Vec<String> = (0..80).map(|n| format!("turn{n:02} text")).collect();
    let mut app = turned(&bodies.iter().map(String::as_str).collect::<Vec<_>>());
    let key = key_of(&app, 79);
    app.model.set_turn(Some(key));
    let (buf, _, y) = shown(&mut app, "turn79");
    let left = app.layout.map.region(select::Panel::Lane).map(|r| r.area.x);
    let left = left.expect("a lane region");
    assert!(matches!(buf[(left, y)].symbol(), "│" | "|"), "the bar");
    let bars = (0..buf.area.height).filter(|row| matches!(buf[(left, *row)].symbol(), "│" | "|"));
    for row in bars {
        let cell = ratatui_core::layout::Position::new(left, row);
        let blank = app.layout.map.blanks.iter().any(|b| b.contains(cell));
        assert!(blank, "the bar on row {row} is a recorded blank");
    }
}

/// Every session row the list draws is inside the list's region, the last
/// one included.
#[test]
fn the_list_region_holds_every_drawn_session_row() {
    let mut app = turned(&["alpha"]);
    app.fleet = Fleet {
        rows: vec![row("api", 1, true), row("zzzlast", 2, false)],
        home: Some("api".to_owned()),
    };
    app.model = Model::new(&app.fleet);
    let (_, x, y) = shown(&mut app, "zzzlast");
    let at = ratatui_core::layout::Position::new(x, y);
    let panel = app.layout.map.region_at(at).map(|region| region.panel);
    assert_eq!(panel, Some(select::Panel::List), "the last drawn row");
}

/// The bar a marked turn wears is chrome: the cell is a recorded blank,
/// and dragging across it copies the very bytes the unmarked turn gives.
#[test]
fn a_marked_turns_bar_is_never_copied() {
    let copied = |marked: bool| {
        let mut app = turned(&["alpha beta", "other"]);
        if marked {
            let key = key_of(&app, 0);
            app.model.set_turn(Some(key));
        }
        let (buf, x, y) = shown(&mut app, "alpha");
        let left = app.layout.map.region(select::Panel::Lane).map(|r| r.area.x);
        let left = left.expect("a lane region");
        let bar = app.layout.map.blanks.iter().find(|blank| blank.y == y);
        assert_eq!(bar.is_some(), marked, "only a marked turn has a bar");
        if marked {
            assert!(matches!(buf[(left, y)].symbol(), "│" | "|"), "the bar");
        }
        app.mouse(at(MouseKind::Click, left, y));
        app.mouse(at(MouseKind::Drag, x + 4, y));
        app.mouse(at(MouseKind::Release, x + 4, y));
        let _ = shown(&mut app, "alpha");
        staged(&app).map(str::to_owned)
    };
    let (marked, plain) = (copied(true), copied(false));
    assert_eq!(marked, plain, "marked and unmarked bytes agree");
    assert!(marked.is_some_and(|text| text.ends_with("alpha")));
}

/// A press whose release never came is not a selection: it holds nothing
/// for Esc, which closes the panel as it always did.
#[test]
fn esc_after_a_press_with_no_motion_keeps_its_usual_job() {
    let (mut app, _asks) = peeker();
    let _ = pressed(&mut app, b"v");
    app.peek.shown = Some(super::peek::Body::Lines(vec!["alphaZ".to_owned()]));
    let (_, x, y) = shown(&mut app, "alphaZ");
    app.mouse(at(MouseKind::Click, x, y));
    assert!(app.select.is_some(), "the press is armed");
    let _ = take_keys(&mut app, vec![(Key::Escape, Instant::now())]);
    assert!(!app.model.peek_open(), "Esc closed the panel");
    assert!(app.select.is_none(), "and dropped the arm");
}

/// A drag whose motion reports were coalesced away - the release lands on
/// another cell - is still a drag: the mark its press toggled goes back.
#[test]
fn a_release_on_another_cell_puts_back_the_mark_the_press_toggled() {
    let mut app = turned(&["alpha beta gamma"]);
    let (_, x, y) = shown(&mut app, "alpha");
    app.mouse(at(MouseKind::Click, x, y));
    assert!(app.model.turn().is_some(), "the press marked the turn");
    app.mouse(at(MouseKind::Release, x + 4, y));
    assert_eq!(app.model.turn(), None, "a drag leaves the mark as it was");
    let _ = shown(&mut app, "alpha");
    assert_eq!(staged(&app), Some("alpha"));
}

/// A view handed over for `web`, written now.
fn handed(pages: usize, rows: usize) -> Answer {
    let view = super::handover::View {
        session: "web".to_owned(),
        id: ID.to_owned(),
        focus: None,
        sidebar: None,
        list: None,
        pages,
        rows,
        list_top: None,
        body_top: 0,
        panel: None,
    };
    let at = Timestamp::now().epoch();
    Answer::Handover { view, at }
}

/// A raw read that decodes to no key - a lone Esc, a paste start, a
/// dropped `^Z` - still ends a handed-over view, waiting or yet to come.
#[test]
fn raw_input_that_decodes_to_no_key_cancels_a_handed_over_view() {
    for raw in [&b"\x1b"[..], b"\x1b[200~", b"\x1a"] {
        for waiting in [true, false] {
            let root = Root::new("raw-cancel");
            session(&root, "web", "");
            let reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
            let mut app = App::new(Some("api".to_owned()), None, None, None);
            let (wake, wakes) = std::sync::mpsc::channel();
            let mut keys = crate::console::input::Keys::app();
            let sent = loader::Wake::Keys(Instant::now(), raw.to_vec());
            let late = loader::Wake::Answer(Box::new(handed(0, 0)));
            if waiting {
                app.answer(handed(0, 0));
                wake.send(sent).expect("sent");
            } else {
                wake.send(sent).expect("sent");
                wake.send(late).expect("sent");
            }
            assert!(super::drain(&mut app, &mut keys, &wakes, &mut None).is_some());
            assert!(
                app.touched && app.keep.is_none(),
                "{raw:?} waiting={waiting}"
            );
            let live = [("api", Status::Running), ("web", Status::Running)];
            refold(&mut app, &reader, &root, &live);
            assert_eq!(app.model.selected(), Some("api"), "{raw:?} no late yank");
        }
    }
}

/// A restored scroll waits for the lane it scrolls: the frame before the
/// session's first read keeps it, and the read then bounds it as ever.
#[test]
fn a_restored_scroll_survives_the_frame_before_its_lane_is_read() {
    let root = Root::new("keep-scroll");
    session(&root, "web", "");
    let reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
    let mut app = App::new(Some("api".to_owned()), None, None, None);
    app.answer(handed(2, 3));
    let live = [("api", Status::Running), ("web", Status::Running)];
    refold(&mut app, &reader, &root, &live);
    assert_eq!(app.model.selected(), Some("web"));
    assert!(app.loading, "web's lane is not read yet");
    let _ = framed(&mut app);
    assert_eq!(app.model.scroll_rows(10), 23, "loading frame keeps it");
    app.answer(Answer::View(ViewRead {
        name: "web".to_owned(),
        id: ID.to_owned(),
        seq: 1,
        lane: Lane {
            items: (0..400).map(|at| said(&format!("turn {at}"), at)).collect(),
            coverage: Vec::new(),
        },
        needs: None,
        roster: None,
    }));
    let _ = framed(&mut app);
    assert_ne!(app.model.scroll_rows(10), 0, "a long history keeps it");
    assert!(!app.loading);
}

/// Cold `2q` arms quit after queueing the 2: its replay keeps the press.
#[test]
fn a_press_armed_before_the_fleet_survives_the_replay_of_the_keys_before_it() {
    let root = Root::new("early-armed");
    session(&root, "web", "");
    let reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
    let mut app = App::new(Some("api".to_owned()), None, None, None);
    let at = Instant::now();
    let _ = take_keys(&mut app, vec![(Key::Text(b"2q".to_vec()), at)]);
    let live = [("api", Status::Running), ("web", Status::Running)];
    refold(&mut app, &reader, &root, &live);
    assert_eq!(app.model.selected(), Some("web"), "the 2 was replayed");
    let again = vec![(Key::Text(b"q".to_vec()), at + Duration::from_millis(1))];
    assert!(take_keys(&mut app, again).is_none(), "the second q quits");
}

/// A handed-over view finds its session when that is the fleet's only row.
#[test]
fn a_handed_over_view_restores_on_a_fleet_whose_only_row_is_its_session() {
    let root = Root::new("sole-row");
    session(&root, "web", "");
    let reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
    let mut app = App::new(Some("api".to_owned()), None, None, None);
    app.answer(handed(2, 3));
    refold(&mut app, &reader, &root, &[("web", Status::Running)]);
    assert_eq!(app.model.selected(), Some("web"));
    assert_eq!(app.model.scroll_rows(10), 23, "and the scroll");
    assert!(app.keep.is_none(), "applied once");
}

/// Once its lane is read, a restored scroll is bound by it and waits no more.
#[test]
fn a_restored_scroll_is_bounded_by_the_lane_once_it_is_read() {
    let root = Root::new("bound-scroll");
    session(&root, "web", "");
    let reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
    let mut app = App::new(Some("api".to_owned()), None, None, None);
    app.answer(handed(2, 3));
    let live = [("api", Status::Running), ("web", Status::Running)];
    refold(&mut app, &reader, &root, &live);
    app.answer(view("web", ID, 1, "short"));
    let _ = framed(&mut app);
    assert_eq!(app.model.scroll_rows(10), 0, "a short lane bounds it");
    assert!(!app.restored, "and the wait is over");
}

/// A key dropped before the first fleet read says so on the hint row now.
#[test]
fn a_key_dropped_before_the_first_fleet_read_repaints_its_notice() {
    let mut app = App::new(Some("api".to_owned()), None, None, None);
    let dropped = take_keys(&mut app, vec![(Key::Text(b"v".to_vec()), Instant::now())]);
    assert_eq!(dropped, Some(true), "the notice repaints before the fleet");
    let said = "input before sessions loaded was dropped";
    assert_eq!(app.note().as_deref(), Some(said));
}

/// Like [`refold`], but each world entry carries its own answer epoch.
fn refold_answered(app: &mut App, reader: &Reader, root: &Root, live: &[(&str, Option<i64>)]) {
    let dir = |name: &str| (name.to_owned(), root.0.join("sessions").join(name));
    let dirs = live.iter().map(|(name, _)| dir(name)).collect();
    let world = live.iter().map(|(name, answered)| {
        let mut e = entry(name, Status::Running, None);
        e.answer_epoch = *answered;
        e
    });
    let world = World::new(Timestamp::now(), world.collect());
    let order = crate::theme::FleetOrder::EMPTY;
    let read = reader.fold(dirs, world, None, &order, Timestamp::now());
    app.answer(Answer::Fleet(read));
}

/// Select by row name, through the digit key its index owns.
fn select(app: &mut App, name: &str) {
    let rows = &app.fleet.rows;
    let index = rows.iter().find(|row| row.name == name).unwrap().index;
    let digit = u8::try_from(index).unwrap_or(0);
    app.model
        .key(super::model::Key::Digit(digit), &app.fleet, false);
    assert_eq!(app.model.selected(), Some(name));
}

/// First sight baselines; a strictly newer answer marks until its session
/// is selected; home never marks. New answers use now+3600: every frame-now
/// in this test is smaller, so `>` is deterministic, never a same-second flake.
#[test]
fn unread_marks_a_new_answer_until_its_session_is_selected() {
    let root = Root::new("cue");
    let (mut app, reader) = housed(&root);
    let now = Timestamp::now().epoch();
    let live = |beta| [("api", Some(now + 3600)), ("beta", beta)];
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
    let marked = |app: &mut App, buf: &mut Buffer, name: &str| {
        app.frame(buf);
        app.model.is_unread(name)
    };
    refold_answered(&mut app, &reader, &root, &live(Some(now - 100)));
    assert_eq!(app.model.selected(), Some("api"));
    assert!(!marked(&mut app, &mut buf, "beta"), "first sight baselines");
    assert!(!marked(&mut app, &mut buf, "api"), "home never marks");
    select(&mut app, "beta");
    marked(&mut app, &mut buf, "beta");
    refold_answered(&mut app, &reader, &root, &live(Some(now + 3600)));
    assert!(!marked(&mut app, &mut buf, "beta"), "selected never marks");
    select(&mut app, "api");
    assert!(marked(&mut app, &mut buf, "beta"), "a newer answer marks");
    assert!(!marked(&mut app, &mut buf, "api"), "home never marks");
    select(&mut app, "beta");
    assert!(!marked(&mut app, &mut buf, "beta"), "selecting clears");
}

/// quotacache P1-P3: a quota scope with one drawn window. Base API only;
/// the pins read paint text, never the kept marker the product adds.
fn quotacache_scope(title: &str) -> crate::quota::view::Scope {
    crate::quota::view::Scope {
        title: title.to_owned(),
        profiles: "a".to_owned(),
        account: None,
        only: None,
        windows: vec![crate::quota::view::Window {
            name: "w5h".to_owned(),
            raw: Some("31%".to_owned()),
            counts: None,
            fill: Some(2),
            why: None,
            level: crate::quota::view::Level::Headroom,
            stale: false,
            resets: Some("2h10m".to_owned()),
            seen: Some("2m".to_owned()),
            burn: None,
        }],
    }
}

/// quotacache P1-P3: an app that answered once, closed, and reopened.
fn quotacache_reopened() -> App {
    let mut app = app(Some("api"));
    app.open_settings();
    app.answer(Answer::Settings {
        generation: 1,
        quota: vec![quotacache_scope("qrow-retained")],
        config: None,
        about: None,
        instructions: None,
        live_seats: None,
    });
    app.close_settings();
    app.open_settings();
    app
}

/// seatjoin #242 s2 (frozen): live seats ride `settled()`'s generation gate.
/// Wrong: kept outside the gate, a stale answer merged, or cleared on reopen.
#[test]
fn seatjoin_live_seats_follow_the_settings_generation() {
    use crate::quota::{LiveGroup, LiveRow, LiveSeats};
    // `None` is a refresh that failed outright: no quota, no seat snapshot.
    let answer = |app: &mut App, generation, seats: Option<&[&str]>| {
        let row = |seat: &&str| LiveRow {
            session_seat: (*seat).to_owned(),
            ..LiveRow::default()
        };
        let live_seats = seats.map(|seats| LiveSeats {
            groups: vec![LiveGroup {
                rows: seats.iter().map(row).collect(),
                ..LiveGroup::default()
            }],
            incomplete: false,
        });
        let quota = match seats {
            Some(_) => vec![quotacache_scope("qrow")],
            None => crate::quota::view::unavailable(),
        };
        app.answer(Answer::Settings {
            generation,
            quota,
            config: None,
            about: None,
            instructions: None,
            live_seats,
        });
    };
    let mut app = app(Some("api"));
    app.open_settings();
    //Accounts-tab routing (human b717ce77): the seat pins read the dedicated
    // tab; every assertion keeps its original spelling.
    app.model
        .show_settings_tab(crate::app::model::SettingsTab::Accounts);
    answer(&mut app, 1, Some(&["old:lead"][..]));
    assert!(framed(&mut app).contains("old:lead"), "own answer draws");
    app.close_settings();
    app.open_settings();
    app.model
        .show_settings_tab(crate::app::model::SettingsTab::Accounts);
    let kept = framed(&mut app);
    assert!(kept.contains("last updated"), "kept line: {kept}");
    assert!(kept.contains("old:lead"), "kept seat: {kept}");
    answer(&mut app, 1, Some(&["stale:lead"][..]));
    let stale = framed(&mut app);
    assert!(stale.contains("old:lead"), "{stale}");
    assert!(!stale.contains("stale:lead"), "stale answer drawn: {stale}");
    answer(&mut app, 2, Some(&["new:lead"][..]));
    answer(&mut app, 2, None);
    let failed = framed(&mut app);
    assert!(failed.contains("refresh failed"), "{failed}");
    assert!(failed.contains("new:lead"), "failure keeps seats: {failed}");
    assert!(
        !failed.contains("old:lead"),
        "replaced, not merged: {failed}"
    );
    answer(&mut app, 2, Some(&[]));
    let fresh = framed(&mut app);
    assert!(!fresh.contains("new:lead"), "fresh empty drops it: {fresh}");
}

/// P1: reopen paints the kept quota snapshot at once, with a kept line
/// saying its age, before any new answer lands. Base clears the body on
/// open, so the reopened frame says loading. (Reader scheduling stays
/// covered by the barrier/channel pins; this pins App paint.)
#[test]
fn reopen_paints_the_kept_quota_snapshot_at_once() {
    let mut app = quotacache_reopened();
    let shown = framed(&mut app);
    assert!(shown.contains("qrow-retained"), "kept rows paint:\n{shown}");
    assert!(
        shown.contains("last updated"),
        "kept line names the age:\n{shown}"
    );
    assert!(
        shown.contains("— refreshing"),
        "kept line says refreshing:\n{shown}"
    );
    assert!(!shown.contains("loading"), "no loading on reopen:\n{shown}");
}

/// P2: a closed generation changes nothing kept; the new open's first own
/// answer replaces the kept body whole and the kept line is gone.
#[test]
fn closed_answers_keep_nothing_and_the_own_answer_replaces_whole() {
    let mut app = quotacache_reopened();
    app.answer(Answer::Settings {
        generation: 1,
        quota: vec![quotacache_scope("qrow-fresh")],
        config: None,
        about: None,
        instructions: None,
        live_seats: None,
    });
    let kept = framed(&mut app);
    assert!(
        kept.contains("qrow-retained"),
        "closed answer changes nothing:\n{kept}"
    );
    app.answer(Answer::Settings {
        generation: 2,
        quota: vec![quotacache_scope("qrow-fresh")],
        config: None,
        about: None,
        instructions: None,
        live_seats: None,
    });
    let fresh = framed(&mut app);
    assert!(fresh.contains("qrow-fresh"), "own answer paints:\n{fresh}");
    assert!(!fresh.contains("qrow-retained"), "replaced whole:\n{fresh}");
    assert!(!fresh.contains("refreshing"), "kept line gone:\n{fresh}");
}

/// P3: a second reopen before any own answer keeps the body, the line and the
/// ORIGINAL epoch; a closed generation's answer cannot move it.
#[test]
fn a_second_reopen_keeps_the_original_quota_epoch() {
    let mut app = quotacache_reopened();
    let past = Timestamp::now().epoch() - 185;
    app.quota_at = Some(past);
    app.close_settings();
    app.open_settings();
    app.answer(Answer::Settings {
        generation: 2,
        quota: vec![quotacache_scope("qrow-closed")],
        config: None,
        about: None,
        instructions: None,
        live_seats: None,
    });
    let shown = framed(&mut app);
    let line = "last updated 3m ago — refreshing";
    // Kept rows, aged from the original epoch; the closed answer is dropped.
    assert!(shown.contains("qrow-retained"), "{shown}");
    assert!(shown.contains(line), "{shown}");
    assert!(!shown.contains("qrow-closed"), "{shown}");
    assert_eq!(app.quota_at, Some(past), "the epoch is the original");
}

/// A read that fails outright keeps the held values under a line saying so,
/// aged from the original epoch; the next real answer replaces them whole.
/// With nothing held the failure is the body, as before.
#[test]
fn a_failed_quota_refresh_keeps_the_held_values_and_says_so() {
    let answer = |app: &mut App, generation, quota| {
        app.answer(Answer::Settings {
            generation,
            quota,
            config: None,
            about: None,
            instructions: None,
            live_seats: None,
        });
    };
    let mut cold = app(Some("api"));
    let mut app = quotacache_reopened();
    let past = Timestamp::now().epoch() - 185;
    app.quota_at = Some(past);
    answer(&mut app, 2, crate::quota::view::unavailable());
    let failed = framed(&mut app);
    assert!(failed.contains("qrow-retained"), "values kept:\n{failed}");
    let line = "refresh failed · showing values from 3m ago";
    assert!(failed.contains(line), "{failed}");
    assert!(!failed.contains("quota: unavailable"), "{failed}");
    assert_eq!(app.quota_at, Some(past), "a failure moves no epoch");
    answer(&mut app, 2, vec![quotacache_scope("qrow-fresh")]);
    let fresh = framed(&mut app);
    assert!(fresh.contains("qrow-fresh"), "{fresh}");
    assert!(!fresh.contains("qrow-retained") && !fresh.contains("refresh failed"));
    cold.open_settings();
    answer(&mut cold, 1, crate::quota::view::unavailable());
    let shown = framed(&mut cold);
    assert!(shown.contains("quota: unavailable"), "{shown}");
    assert!(!shown.contains("refresh failed"), "{shown}");
}
#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "the test inspects its own fixture files"
)]
fn imgtoken_fleet_discard_releases_owned_images_but_kept_drafts_and_user_files_survive() {
    use std::fs;

    for replaced in [false, true] {
        let root = Root::new("imgtoken-discard");
        let (mut app, _reader) = housed(&root);
        writing(&mut app);
        let owned = root.0.join("owned.png");
        let user = root.0.join("user.pdf");
        fs::write(&owned, b"owned fixture").unwrap();
        fs::write(&user, b"pre-existing user file").unwrap();
        let key = app.writer.as_ref().unwrap().key();
        let input = app.inputs.get_mut(&key).unwrap();
        input.image_path(owned.to_str().unwrap()).unwrap();
        input.restore(crate::console::submit::Draft::Kept(
            format!(" [File: \"{}\"]", user.display()).into_bytes(),
        ));
        app.write(false);
        assert!(owned.exists(), "Esc keeps its draft and image");
        assert!(app.inputs.contains_key(&key));
        assert!(app.inputs.get(&key).unwrap().draft().contains("[Image #1]"));
        let mut unread = read_of(&app, &[]);
        unread.scanned = false;
        app.answer(Answer::Fleet(unread));
        assert!(app.inputs.contains_key(&key), "incomplete read keeps draft");
        assert!(app.inputs.get(&key).unwrap().draft().contains("[Image #1]"));
        assert!(owned.exists(), "uncertain incarnation keeps owned image");
        let ids = if replaced {
            vec![("api", "replacement-id")]
        } else {
            Vec::new()
        };
        let complete = read_of(&app, &ids);
        app.answer(Answer::Fleet(complete));
        assert!(
            !app.inputs.contains_key(&key),
            "complete proof discards draft"
        );
        assert!(
            !owned.exists(),
            "discarded owned image released: replaced={replaced}"
        );
        assert_eq!(
            fs::read(&user).unwrap(),
            b"pre-existing user file",
            "unowned file untouched"
        );
    }
}

// agentwrap frozen behavioral acceptance — r2 fragment for src/app.rs
// INSERT: immediately before the closing brace of the integration tests module
// of src/app.rs that contains bound_app / framed / seat_cell (the module holding
// a_delayed_seat_click_after_removal_refuses_by_name_and_opens_nothing, app.rs:8634).
// Supersedes frozen/r1/app-agentwrap-tests.rs (kept immutable). Oracle r2 fix:
// B4 shown.roster holds SeatRef{slot, name} — no model/effort fields — so the
// label mutation touches ONLY app.facts (the Screen's Facts::Seats source);
// the r1 block that mutated shown.roster is gone.
//
// Derived geometry (binding): bound_app seats carry state "working" (7 cells,
// app.rs:8579). Sidebar pinned to 44 -> room 40; names aa/bb -> column 2 ->
// start 6 -> free = 40 - 6 - 8 = 26. The label is 29 cells > 26, so it must
// wrap; row-2 capacity 38 >= 29, so row two carries it whole.

/// R13/R14 + I4: navigation stays logical across a wrapped seat, proven
/// against a REALLY wrapped buffer: the precondition demands the label as its
/// own painted line and exactly one logical drawn entry, so a missing second
/// row fails the test before navigation can pass falsely.
#[test]
fn navigation_steps_over_a_wrapped_seat_in_one_logical_step() {
    const LABEL: &str = "m1234567890123456789012345678";
    let (_root, mut app, _asks) = bound_app(
        "agentwrap-nav-logical",
        &["api"],
        &[("main", "aa"), ("worker.0", "bb")],
    );
    app.model.grab(super::model::Drag {
        edge: super::model::Edge::Sidebar,
        from: 0,
        at: 0,
    });
    let _ = app.model.drag_to(44);
    let _ = app.model.let_go();
    if let Some(fleet::Facts::Seats { agents, .. }) = app.facts.get_mut("api") {
        for agent in agents.iter_mut().filter(|agent| agent.name == "aa") {
            agent.model = LABEL.to_owned();
            agent.effort = String::new();
        }
    }
    let text = framed(&mut app);
    // Precondition: the seat really wrapped — the label paints as its own line
    // and the seat is still ONE logical drawn entry.
    assert_eq!(
        text.lines().filter(|l| l.contains(LABEL)).count(),
        1,
        "precondition: the label has its own painted row"
    );
    assert_eq!(
        app.drawn
            .as_ref()
            .map(|drawn| drawn.rows.iter().filter(|(name, _)| name == "aa").count()),
        Some(1),
        "precondition: one logical seat, deduped"
    );
    // The continuation row hits its seat like the head row does.
    let row = text
        .lines()
        .enumerate()
        .find_map(|(row, l)| l.contains(LABEL).then_some(row))
        .expect("the label row");
    let hit = app.layout.hit(super::Mouse {
        kind: super::MouseKind::Click,
        column: 8,
        row: u16::try_from(row).expect("frame row"),
    });
    assert!(
        matches!(hit, Some(super::draw::Hit::Seat(name)) if name == "aa"),
        "the continuation row hits its seat"
    );
    // n steps from the wrapped seat to the NEXT seat, not to its own row two;
    // p steps back; n from the last seat wraps to the first.
    app.model.set_focus(Some("aa".to_owned()));
    let _ = framed(&mut app);
    assert!(app.move_focus(true), "n moves");
    assert_eq!(app.model.focus(), Some("bb"), "one logical step to bb");
    assert!(app.move_focus(false), "p moves");
    assert_eq!(app.model.focus(), Some("aa"), "back to the wrapped seat");
    app.model.set_focus(Some("bb".to_owned()));
    let _ = framed(&mut app);
    assert!(app.move_focus(true), "n moves from the last seat");
    assert_eq!(app.model.focus(), Some("aa"), "it wraps to the first seat");
}

#[test]
fn bang_app_destination_is_none_and_effects_handle_shell_and_write_keys_lists_it() {
    let root = Root::new("bang-app");
    let dir = root.0.join("sessions").join("api");
    let work = root.0.join("work");
    std::fs::create_dir_all(&work).expect("work dir");
    let (mut app, _reader) = housed(&root);
    app.pair = vec!["lead".to_owned(), "colead".to_owned()];
    app.draft = "!git status".to_owned();
    assert_eq!(app.destination("api"), None, "a ! line has no seat address");

    app.open_settings();
    app.model.show_settings_tab(SettingsTab::Keys);
    app.model.settings_wheel(15);
    let keys_shown = framed(&mut app);
    assert!(
        keys_shown.contains("!command    run it: a popup in tmux, else started"),
        "{keys_shown}"
    );
    app.close_settings();

    // Without a work_dir in meta, ! is refused while writing stays live and
    // the raw line is put back once into the active writer's composer.
    let at = writing(&mut app);
    for key in [Key::Text(b"!  pwd  ".to_vec()), Key::Enter] {
        assert_eq!(app.compose(key, at), Some(()));
    }
    assert_eq!(
        app.notices.last().map(|(_, item)| item.body.as_str()),
        Some("refused: ! pwd: the session records no work dir")
    );
    assert!(app.composing(), "a shell refusal keeps the writer live");
    assert_eq!(drafted(&app), "!  pwd  ", "raw line restored once");
    let _ = app.compose(Key::ClearLine, at);

    // Missing work_dir on disk is refused before starting `/bin/sh` or `tmux`.
    let gone = root.0.join("gone-work");
    let write_work_meta = |path: &std::path::Path| {
        let text = format!(
            "schema=2\nsession_id={ID}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\nwork_dir={}\n",
            path.display()
        );
        std::fs::write(dir.join("meta"), text).expect("meta with work_dir");
    };
    write_work_meta(&gone);
    for key in [Key::Text(b"!pwd".to_vec()), Key::Enter] {
        assert_eq!(app.compose(key, at), Some(()));
    }
    let expected_gone = format!("refused: ! pwd: its work dir {} is gone", gone.display());
    assert_eq!(
        app.notices.last().map(|(_, item)| item.body.as_str()),
        Some(expected_gone.as_str())
    );
    assert_eq!(drafted(&app), "!pwd");
    let _ = app.compose(Key::ClearLine, at);

    // Inside a tmux pane without a resolved server, or when the server's
    // clients do not answer, ! is refused by name and restores the draft.
    write_work_meta(&work);
    app.pane = Some("%1".to_owned());
    app.server = None;
    for key in [Key::Text(b"!pwd".to_vec()), Key::Enter] {
        assert_eq!(app.compose(key, at), Some(()));
    }
    assert_eq!(
        app.notices.last().map(|(_, item)| item.body.as_str()),
        Some("refused: ! pwd: tmux did not answer")
    );
    assert_eq!(drafted(&app), "!pwd");
    let _ = app.compose(Key::ClearLine, at);

    app.server = Some(crate::inventory::ServerId::Selected(
        crate::meta::Selector::Socket(root.0.join("absent.sock")),
    ));
    for key in [Key::Text(b"!pwd".to_vec()), Key::Enter] {
        assert_eq!(app.compose(key, at), Some(()));
    }
    assert_eq!(
        app.notices.last().map(|(_, item)| item.body.as_str()),
        Some("refused: ! pwd: the tmux clients did not answer")
    );
    assert_eq!(drafted(&app), "!pwd");
    let _ = app.compose(Key::ClearLine, at);
}

#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "the test inspects its own fixture files"
)]
fn bang_app_outside_tmux_runs_in_work_dir_records_nothing_and_unproven_restores_once() {
    let root = Root::new("bang-app-outside");
    let dir = root.0.join("sessions").join("api");
    let work = root.0.join("work");
    std::fs::create_dir_all(&work).expect("work dir");
    let (mut app, _reader) = housed(&root);
    app.pair = vec!["lead".to_owned(), "colead".to_owned()];
    let at = writing(&mut app);
    let text = format!(
        "schema=2\nsession_id={ID}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\nwork_dir={}\n",
        work.display()
    );
    std::fs::write(dir.join("meta"), text).expect("meta with work_dir");
    app.pane = None;
    app.server = None;

    // Outside tmux (`app.pane` is None) with a live work_dir, ! runs via
    // `/bin/sh -c` in `work_dir` and records nothing in events.jsonl,
    // messages/, memo.tsv, console.draft, or Telegram (outbound Telegram
    // reads events.jsonl and has no separate outbox file).
    let out_file = work.join("app-bang.txt");
    let cmd = format!(
        "!printf '%s\\n' \"$(pwd -P)\" > {0}.tmp && mv {0}.tmp {0}",
        out_file.display()
    );
    for key in [Key::Text(cmd.as_bytes().to_vec()), Key::Enter] {
        assert_eq!(app.compose(key, at), Some(()));
    }
    let expected_started = format!("! {} - started", &cmd[1..]);
    assert_eq!(
        app.notices.last().map(|(_, item)| item.body.as_str()),
        Some(expected_started.as_str())
    );
    assert_eq!(drafted(&app), "", "composer cleared after started shell");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !out_file.exists() {
        std::thread::sleep(Duration::from_millis(20));
    }
    let recorded_pwd = std::fs::read_to_string(&out_file).expect("shell output file");
    assert_eq!(
        std::path::PathBuf::from(recorded_pwd.trim()),
        work.canonicalize().expect("canonical work dir")
    );
    let store = crate::store::open(&dir);
    assert!(matches!(
        store.events_source(),
        crate::store::SourceRead::Absent
    ));
    assert!(matches!(
        store.memo_source(),
        crate::store::SourceRead::Absent
    ));
    assert_eq!(super::submit::restore(&dir), super::submit::Draft::Nothing);
    assert!(
        !dir.join("messages").exists() && !root.0.join("telegram").exists(),
        "no message body or Telegram state written"
    );

    // When home ownership is lost before Enter, `self.owns(&lost)` fails and
    // `self.unproven(lost, &raw)` ends writing and restores `raw` once (never twice).
    meta(&root, &ID.replace("1234", "bbbb"), "colead");
    for key in [Key::Text(b"!echo keep-once".to_vec()), Key::Enter] {
        assert_eq!(app.compose(key, at), Some(()));
    }
    assert!(!app.composing(), "unproven home ends writing");
    assert_eq!(
        app.notices.last().map(|(_, item)| item.body.as_str()),
        Some("refused: the session was replaced or renamed")
    );
    assert_eq!(
        app.inputs.values().next().expect("input").draft(),
        "!echo keep-once",
        "unproven restores raw once without a second put_back"
    );
}
