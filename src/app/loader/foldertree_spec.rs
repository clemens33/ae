//! Independent acceptance for #279 part 1, from the issue and foldertree brief.
//! Install byte-identically and wire `#[cfg(test)] mod foldertree_spec;`
//! inside app/loader.rs. Decorations and numeric cap choices remain product choices.
//! The companion live FIFO/UI case is `tests/it/foldertree_fifo_spec.rs`.
//! Names sort by raw name bytes within each kind, including case (B before a).
//! Residual: `terminal_text` alone preserves bidi controls, which can reorder a row.
//! These tests permit additional per-name neutralisation without fixing its spelling.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "independent acceptance owns and inspects only its temporary fixtures"
)]

use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::{Answer, ReadAsk, Reader, Request};
use crate::app::filedoor::{TREE_MAX_DEPTH, TREE_MAX_ENTRIES};
use crate::app::fleet::{Counts, Fleet, Line2, Row};
use crate::app::model::Model;
use crate::app::reader::Body;
use crate::app::{App, take_keys};
use crate::console::input;
use crate::console::lane::{Item, Kind, Lane};
use crate::console::link::Base;
use crate::digest::{SessionEntry, Status};
use crate::theme::Mark;
use crate::time::Timestamp;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "ae-foldertree-spec-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("new owned fixture");
        Self(root)
    }

    fn dir(&self, name: &str) -> PathBuf {
        let dir = self.0.join(name);
        fs::create_dir_all(&dir).expect("owned folder");
        dir
    }

    fn base(&self) -> Base {
        Base {
            dir: Some(self.dir("work").to_string_lossy().into_owned()),
            home: Some(self.dir("home").to_string_lossy().into_owned()),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn ask(path: &Path, base: Base) -> ReadAsk {
    ReadAsk {
        generation: 279,
        path: path.to_string_lossy().into_owned(),
        line: None,
        links: 1,
        base,
    }
}

fn load(ask: &ReadAsk) -> Body {
    let mut reader = Reader::new(PathBuf::from("/nonexistent"), None, None);
    let Answer::Read { generation, body } = reader.read(ask) else {
        panic!("a read request must answer a read");
    };
    assert_eq!(generation, ask.generation);
    body
}

fn tree(body: &Body) -> &[String] {
    let Body::Tree { lines } = body else {
        panic!("an allowed folder must be a tree, got {body:?}");
    };
    lines
}

fn at(lines: &[String], name: &str) -> usize {
    lines
        .iter()
        .position(|line| line.contains(name))
        .unwrap_or_else(|| panic!("missing {name:?}: {lines:?}"))
}

fn named_cut(lines: &[String]) -> bool {
    lines.iter().any(|line| {
        let text = line.to_lowercase();
        text.contains("more")
            || text.contains("limit")
            || text.contains("truncated")
            || text.contains('…')
    })
}

#[test]
fn folders_under_either_canonical_root_are_trees_empty_ones_included() {
    let f = Fixture::new();
    let base = f.base();
    for path in [f.dir("work/preview"), f.dir("home/preview")] {
        let body = load(&ask(&path, base.clone()));
        let lines = tree(&body);
        assert!(lines.len() <= 3, "an empty tree stays small: {lines:?}");
    }
    let root_alias = f.0.join("work-alias");
    symlink(f.dir("work"), &root_alias).expect("root alias");
    let base = Base {
        dir: Some(root_alias.to_string_lossy().into_owned()),
        home: None,
    };
    let child = f.dir("work/child");
    fs::write(child.join("canonical-root.txt"), b"content").expect("owned file");
    let body = load(&ask(&child, base));
    at(tree(&body), "canonical-root.txt");
}

#[test]
fn a_linked_folder_alias_inside_a_root_is_followed_like_an_inside_file_alias() {
    let f = Fixture::new();
    let base = f.base();
    let target = f.dir("work/real-folder");
    fs::write(target.join("INSIDE_ALIAS_CHILD.txt"), b"x").expect("owned child");
    let alias = f.0.join("work/folder-alias");
    symlink(target, &alias).expect("inside folder alias");
    let body = load(&ask(&alias, base));
    at(tree(&body), "INSIDE_ALIAS_CHILD.txt");
}

#[test]
fn canonical_containment_refuses_direct_dotdot_and_symlink_escapes_in_one_gap() {
    let f = Fixture::new();
    let base = f.base();
    let outside = f.dir("outside");
    fs::write(outside.join("secret-child.txt"), b"private").expect("outside file");
    let link = f.0.join("work/escape");
    symlink(&outside, &link).expect("outside folder alias");
    // A prefix-string comparison would wrongly admit work-neighbor.
    let neighbor = f.dir("work-neighbor");
    for path in [outside, f.0.join("work/../outside"), link, neighbor] {
        let asked = ask(&path, base.clone());
        assert_eq!(
            load(&asked),
            Body::Gap(format!(
                "refused: read {}: outside the session dir and home",
                asked.path
            )),
            "the whole folder refuses before any listing"
        );
    }
    let path = f.dir("work/allowed");
    assert_eq!(
        load(&ask(
            &path,
            Base {
                dir: None,
                home: None
            }
        )),
        Body::Gap(format!(
            "refused: read {}: no base to read against",
            path.display()
        ))
    );
}

#[test]
fn symlinks_are_named_link_entries_never_descended_even_inside_the_root() {
    let f = Fixture::new();
    let base = f.base();
    let folder = f.dir("work/tree");
    let inside = f.dir("work/elsewhere");
    let outside = f.dir("outside");
    fs::write(inside.join("INTERNAL_DESCEND_FORBIDDEN"), b"x").expect("inside child");
    fs::write(outside.join("EXTERNAL_DESCEND_FORBIDDEN"), b"x").expect("outside child");
    for (name, target) in [
        ("internal-link", inside),
        ("external-link", outside),
        ("cycle-link", folder.clone()),
        ("broken-link", f.0.join("missing-target")),
    ] {
        symlink(target, folder.join(name)).expect("owned link");
    }
    let body = load(&ask(&folder, base));
    let lines = tree(&body);
    for name in [
        "internal-link",
        "external-link",
        "cycle-link",
        "broken-link",
    ] {
        let line = &lines[at(lines, name)];
        assert!(
            line.contains('@')
                || line.contains("[link]")
                || line.contains("(link)")
                || line.contains('→'),
            "a link is identified as a link: {line}"
        );
    }
    let joined = lines.join("\n");
    assert!(!joined.contains("DESCEND_FORBIDDEN"), "{joined}");
    assert!(lines.len() <= 8, "a cycle must not expand: {lines:?}");
}

#[test]
fn each_level_lists_folders_first_then_names_and_keeps_parent_child_structure() {
    let f = Fixture::new();
    let base = f.base();
    let folder = f.dir("work/tree");
    for name in ["b-dir", "A-dir"] {
        fs::create_dir(folder.join(name)).expect("owned directory");
    }
    for name in ["z-file", "B-file", "a-file"] {
        fs::write(folder.join(name), b"not file content in the tree").expect("owned file");
    }
    fs::write(folder.join("A-dir/deep-child"), b"x").expect("nested file");
    let body = load(&ask(&folder, base));
    let lines = tree(&body);
    let positions: Vec<_> = ["A-dir", "deep-child", "b-dir", "B-file", "a-file", "z-file"]
        .iter()
        .map(|name| at(lines, name))
        .collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "{lines:?}"
    );
    assert!(
        !lines.join("\n").contains("not file content"),
        "files are listed, never opened"
    );
}

#[test]
fn git_and_target_are_visible_collapsed_and_other_dot_folders_are_walked() {
    let f = Fixture::new();
    let base = f.base();
    let folder = f.dir("work/tree");
    for name in [".git", "target", ".config"] {
        fs::create_dir(folder.join(name)).expect("owned directory");
        fs::write(folder.join(name).join(format!("{name}-hidden-child")), b"x")
            .expect("owned child");
    }
    let body = load(&ask(&folder, base));
    let lines = tree(&body);
    for name in [".git", "target"] {
        let row = &lines[at(lines, name)];
        assert!(
            row.contains('…') || row.contains("collapsed") || row.contains("more"),
            "a collapsed folder is named as such: {row}"
        );
        assert!(!lines.join("\n").contains(&format!("{name}-hidden-child")));
    }
    at(lines, ".config-hidden-child");
}

struct RestorePermissions(PathBuf, fs::Permissions);

impl Drop for RestorePermissions {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, self.1.clone());
    }
}

#[test]
fn an_unreadable_child_is_named_without_hiding_readable_siblings_or_refusing_the_tree() {
    let f = Fixture::new();
    // Ownership of the freshly created fixture is the effective creator's uid.
    // Root bypasses mode 000, so that process cannot exercise this fixture.
    if fs::metadata(&f.0).expect("owned fixture metadata").uid() == 0 {
        return;
    }
    let base = f.base();
    let folder = f.dir("work/tree");
    let unreadable = folder.join("unreadable-child");
    fs::create_dir(&unreadable).expect("owned unreadable child");
    fs::write(unreadable.join("HIDDEN_UNREADABLE_CHILD"), b"x").expect("owned hidden file");
    fs::write(folder.join("READABLE_SIBLING.txt"), b"x").expect("owned sibling");
    let original = fs::metadata(&unreadable)
        .expect("child metadata")
        .permissions();
    let _restore = RestorePermissions(unreadable.clone(), original);
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).expect("deny child read");
    assert_eq!(
        fs::read_dir(&unreadable)
            .expect_err("fixture must refuse reads")
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    let body = load(&ask(&folder, base));
    let lines = tree(&body);
    at(lines, "READABLE_SIBLING.txt");
    let row = &lines[at(lines, "unreadable-child")];
    let reason = row.replace("unreadable-child", "");
    assert!(
        reason.contains("unreadable") || reason.contains("denied") || reason.contains("permission"),
        "the child has a short named reason: {row}"
    );
    assert!(row.chars().count() < 250, "the reason stays short");
    assert!(!lines.join("\n").contains("HIDDEN_UNREADABLE_CHILD"));
}

#[test]
fn entry_cap_has_an_exact_named_omission_count_for_a_flat_folder() {
    let f = Fixture::new();
    let base = f.base();
    let folder = f.dir("work/tree");
    for n in 0..TREE_MAX_ENTRIES + 5 {
        fs::write(folder.join(format!("entry-{n:06}")), b"x").expect("owned file");
    }
    let body = load(&ask(&folder, base));
    let lines = tree(&body);
    let shown: Vec<_> = lines
        .iter()
        .filter(|line| line.contains("entry-"))
        .collect();
    assert_eq!(
        shown.len(),
        TREE_MAX_ENTRIES,
        "the published cap applies to entries"
    );
    let names: Vec<_> = shown
        .iter()
        .map(|line| {
            let start = line.find("entry-").expect("selected entry line");
            &line[start..]
        })
        .collect();
    assert!(
        names.windows(2).all(|pair| pair[0] < pair[1]),
        "kept entries sort by name"
    );
    assert!(
        lines.iter().any(|line| line.contains("more")
            && line
                .split(|c: char| !c.is_ascii_digit())
                .any(|word| word == "5")),
        "the cut names the five omitted entries: {lines:?}"
    );
    assert!(
        lines.len() <= TREE_MAX_ENTRIES + 3,
        "markers must also stay bounded"
    );
}

#[test]
fn entry_budget_applies_to_the_whole_tree_instead_of_restarting_in_each_folder() {
    let f = Fixture::new();
    let base = f.base();
    let folder = f.dir("work/tree");
    for branch in ["branch-a", "branch-b"] {
        let child = folder.join(branch);
        fs::create_dir(&child).expect("owned branch");
        for n in 0..TREE_MAX_ENTRIES / 2 + 7 {
            fs::write(child.join(format!("global-entry-{branch}-{n:06}")), b"x")
                .expect("owned entry");
        }
    }
    let body = load(&ask(&folder, base));
    let lines = tree(&body);
    let count = lines
        .iter()
        .filter(|line| {
            line.contains("global-entry-") || line.contains("branch-a") || line.contains("branch-b")
        })
        .count();
    assert!(
        count > 2 && count <= TREE_MAX_ENTRIES,
        "one budget covers every level: {count}"
    );
    assert!(named_cut(lines), "the global cut is visible");
}

#[test]
fn depth_cap_stops_a_real_nested_walk_and_names_the_cut() {
    let f = Fixture::new();
    let base = f.base();
    let folder = f.dir("work/tree");
    let mut next = folder.clone();
    for n in 1..=TREE_MAX_DEPTH + 3 {
        next = next.join(format!("level-{n:03}"));
        fs::create_dir(&next).expect("owned nested directory");
    }
    fs::write(next.join("TOO_DEEP_FOR_PREVIEW"), b"x").expect("deep sentinel");
    let body = load(&ask(&folder, base));
    let lines = tree(&body);
    assert_eq!(
        lines.iter().filter(|line| line.contains("level-")).count(),
        TREE_MAX_DEPTH
    );
    assert!(!lines.join("\n").contains("TOO_DEEP_FOR_PREVIEW"));
    assert!(named_cut(lines), "the depth cut is visible: {lines:?}");
}

#[test]
fn hostile_names_remain_visible_valid_terminal_text_without_inventing_extra_rows() {
    let f = Fixture::new();
    let base = f.base();
    let folder = f.dir("work/tree");
    let names = [
        "escape-\u{1b}[31m",
        "control-\u{7}",
        "newline-\n-forged",
        "tab-\t-name",
        "bidi-\u{202e}-name",
    ];
    for name in names {
        fs::write(folder.join(name), b"x").expect("hostile fixture name");
    }
    let long = format!("long-{}", "x".repeat(235));
    fs::write(folder.join(&long), b"x").expect("long fixture name");
    fs::write(
        folder.join(std::ffi::OsString::from_vec(b"invalid-\xff-name".to_vec())),
        b"x",
    )
    .expect("non UTF-8 fixture name");
    let body = load(&ask(&folder, base));
    let lines = tree(&body);
    for line in lines {
        assert_eq!(
            *line,
            crate::board::terminal_text(line),
            "all names pass the output owner"
        );
        assert!(
            !line.contains(['\n', '\t']),
            "one name stays on one logical row: {line:?}"
        );
    }
    for prefix in [
        "escape-", "control-", "newline-", "tab-", "bidi-", "long-", "invalid-",
    ] {
        at(lines, prefix);
    }
    assert!(
        lines.join("\n").contains('\u{fffd}'),
        "hostile bytes are neutralised visibly"
    );
    assert!(
        lines.len() <= names.len() + 4,
        "a forged newline cannot add an entry"
    );
}

#[test]
fn regular_file_bytes_and_the_decoded_reader_body_are_unchanged() {
    let f = Fixture::new();
    let base = f.base();
    let file = f.0.join("work/plain.txt");
    let bytes = b"alpha\r\nbeta\n\n";
    fs::write(&file, bytes).expect("owned file");
    assert_eq!(
        load(&ask(&file, base)),
        Body::Text {
            lines: vec!["alpha".to_owned(), "beta".to_owned()],
            size: u64::try_from(bytes.len()).expect("small file"),
        }
    );
    assert_eq!(fs::read(&file).expect("read fixture back"), bytes);
}

#[test]
fn a_folder_preview_does_not_modify_its_entries_or_their_bytes() {
    let f = Fixture::new();
    let base = f.base();
    let folder = f.dir("work/tree");
    let written: [(&str, &[u8]); 3] = [
        ("events.jsonl", b"{\"body\":\"unchanged\"}\n"),
        ("console.draft", b"an unfinished thought"),
        ("plain.txt", b"alpha\nbeta\n"),
    ];
    for (name, bytes) in written {
        fs::write(folder.join(name), bytes).expect("owned file");
    }
    let body = load(&ask(&folder, base));
    let lines = tree(&body);
    for (name, bytes) in written {
        at(lines, name);
        assert_eq!(
            fs::read(folder.join(name)).expect("entry still present"),
            bytes
        );
    }
    assert_eq!(
        fs::read_dir(folder).expect("folder still present").count(),
        written.len()
    );
}

fn app(asked: &ReadAsk) -> (App, Receiver<Request>) {
    let (tx, rx) = channel();
    let mut app = App::new(Some("api".to_owned()), None, None, Some(tx));
    (app.fleeted, app.loading) = (true, false);
    app.fleet = Fleet {
        rows: vec![Row {
            name: "api".to_owned(),
            index: 1,
            mark: Mark::Working,
            needy: false,
            unanswered: None,
            counts: Counts::Unknown,
            line2: Line2::NoGoal,
            home: true,
        }],
        home: Some("api".to_owned()),
    };
    app.model = Model::new(&app.fleet);
    let mut entry = SessionEntry::new("api", Status::Running);
    entry.work_dir.clone_from(&asked.base.dir);
    app.world.sessions.push(entry.clone());
    app.overview = crate::app::overview::of(
        &entry,
        None,
        asked.base.home.as_deref().map(Path::new),
        Timestamp::from_epoch(1_759_500_600),
    );
    app.lane = Rc::new(Lane {
        items: vec![Item {
            micros: 1,
            kind: Kind::Said {
                who: "lead".to_owned(),
            },
            body: format!("preview {}", asked.path),
            record: None,
        }],
        coverage: vec![],
    });
    let mark = app.keys()[0];
    app.model.set_turn(Some(mark));
    (app, rx)
}

fn frame(app: &mut App) -> String {
    let mut buf = Buffer::empty(Rect::new(0, 0, 200, 45));
    app.frame(&mut buf);
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn marked_folder_link_only_dispatches_on_ui_then_real_loader_tree_reaches_the_frame() {
    let f = Fixture::new();
    let base = f.base();
    let folder = f.0.join("work/created-after-open");
    let (mut app, rx) = app(&ask(&folder, base));
    let mark = app.model.turn();
    assert_eq!(
        take_keys(
            &mut app,
            vec![(input::Key::Text(b"r".to_vec()), Instant::now())]
        ),
        Some(true)
    );
    let Ok(Request::Read(Some(asked))) = rx.try_recv() else {
        panic!("UI enqueues one read");
    };
    assert_eq!(asked.path, folder.to_string_lossy());
    assert!(rx.try_recv().is_err());
    let pending = frame(&mut app);
    assert!(
        pending.contains("loading"),
        "the UI does not refuse a currently missing folder: {pending}"
    );
    fs::create_dir(&folder).expect("folder appears after UI dispatch");
    fs::write(
        folder.join("AFTER_DISPATCH.txt"),
        b"file contents must not appear",
    )
    .expect("owned file");
    let mut reader = Reader::new(f.0.clone(), None, None);
    app.answer(reader.read(&asked));
    let shown = frame(&mut app);
    assert!(
        shown.contains("Read · ") && shown.contains("AFTER_DISPATCH.txt"),
        "{shown}"
    );
    assert!(
        !shown.contains("not a regular file") && !shown.contains("file contents must not appear")
    );
    assert_eq!(
        take_keys(&mut app, vec![(input::Key::Escape, Instant::now())]),
        Some(true)
    );
    assert!(!app.model.reader_open());
    assert_eq!(app.model.turn(), mark);
    assert!(matches!(rx.try_recv(), Ok(Request::Read(None))));
    assert!(
        rx.try_recv().is_err(),
        "the read-only round trip emits no other request"
    );
}
