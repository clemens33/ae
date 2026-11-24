//! The durable app view file against hostile nodes: the reader never follows a
//! link and never waits on a FIFO, and the writer never replaces or writes
//! through a node that is not a regular file. The restore itself is pinned by
//! `app_view_spec`; this is the file door alone.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the spec reads and writes only its private scratch directory"
)]

use std::os::unix::fs::{FileTypeExt as _, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use ae::app::handover::{self, Handover, View};
use ae::app::saved;

const WEB_ID: &str = "6f1c2b7e-0d3a-4c58-9a2e-5b1d7c9e0a11";

fn held() -> Handover {
    Handover::durable(
        "2026.11.113",
        1_760_259_000,
        View {
            session: "web".into(),
            id: WEB_ID.into(),
            focus: Some("colead".into()),
            sidebar: Some(31),
            list: None,
            pages: 0,
            rows: 0,
            list_top: None,
            body_top: 0,
            panel: None,
            desk: Some(44),
            desk_hidden: true,
            desk_tab: ae::desk::Section::Running,
            zoom: true,
            orchestrator: None,
        },
    )
}

fn scratch(tag: &str) -> PathBuf {
    super::cli::OwnedScratch::root("appviewfile", tag).keep()
}

/// A read on its own thread, so a reader that blocks fails the test instead of
/// hanging it.
fn read_bounded(path: &Path) -> Option<Handover> {
    let (tell, hear) = mpsc::channel();
    let at = path.to_owned();
    std::thread::spawn(move || {
        let _ = tell.send(saved::read_file(&at));
    });
    hear.recv_timeout(Duration::from_secs(20))
        .expect("the reader must not wait on the node")
}

#[test]
fn a_written_view_reads_back_whole_and_is_private() {
    let dir = scratch("roundtrip");
    let file = saved::path(&dir);
    assert_eq!(file, dir.join("app-view"));
    let text = handover::render_file(&held());
    saved::write_file(&file, &text).expect("a fresh file");
    assert_eq!(read_bounded(&file), Some(held()));
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
    let mut again = held();
    again.view.desk_tab = ae::desk::Section::Links;
    saved::write_file(&file, &handover::render_file(&again)).expect("a replacement");
    assert_eq!(read_bounded(&file), Some(again));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_fifo_a_link_a_directory_and_hostile_bytes_read_as_no_file() {
    let dir = scratch("hostile-read");
    let file = dir.join("app-view");
    let valid = handover::render_file(&held());

    crate::cli::mkfifo(&file);
    assert_eq!(
        read_bounded(&file),
        None,
        "a FIFO is refused, never waited on"
    );
    std::fs::remove_file(&file).unwrap();

    let target = dir.join("target");
    std::fs::write(&target, &valid).unwrap();
    symlink(&target, &file).unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        valid,
        "the link leads to a valid view"
    );
    assert_eq!(read_bounded(&file), None, "a link is never followed");
    std::fs::remove_file(&file).unwrap();

    std::fs::create_dir(&file).unwrap();
    assert_eq!(read_bounded(&file), None, "a directory");
    std::fs::remove_dir(&file).unwrap();

    assert_eq!(read_bounded(&file), None, "no file");
    for (what, bytes) in [
        ("empty", Vec::new()),
        ("no line feed", valid.trim_end().as_bytes().to_vec()),
        ("two line feeds", format!("{valid}\n").into_bytes()),
        ("a CRLF", valid.replace('\n', "\r\n").into_bytes()),
        ("a foreign tag", valid.replacen("v3", "v4", 1).into_bytes()),
        ("a NUL", [valid.as_bytes(), b"\0"].concat()),
        ("not UTF-8", [valid.as_bytes(), b"\xff"].concat()),
        ("over the cap", vec![b'x'; handover::FILE_CAP + 1]),
        ("a megabyte", vec![b'x'; 1 << 20]),
    ] {
        std::fs::write(&file, bytes).unwrap();
        assert_eq!(read_bounded(&file), None, "{what}");
    }
    std::fs::write(&file, &valid).unwrap();
    assert_eq!(read_bounded(&file), Some(held()), "the control still reads");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_writer_refuses_a_link_a_directory_and_a_fifo_and_touches_nothing() {
    let dir = scratch("hostile-write");
    let file = dir.join("app-view");
    let text = handover::render_file(&held());

    let target = dir.join("target");
    std::fs::write(&target, "untouched").unwrap();
    symlink(&target, &file).unwrap();
    let refused = saved::write_file(&file, &text).expect_err("a link is refused");
    assert!(refused.contains("a symlink"), "{refused}");
    assert_eq!(std::fs::read_link(&file).unwrap(), target, "the link stays");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "untouched");
    std::fs::remove_file(&file).unwrap();

    std::fs::create_dir(&file).unwrap();
    assert!(saved::write_file(&file, &text).is_err(), "a directory");
    assert!(file.is_dir());
    std::fs::remove_dir(&file).unwrap();

    crate::cli::mkfifo(&file);
    assert!(saved::write_file(&file, &text).is_err(), "a FIFO");
    assert!(
        std::fs::symlink_metadata(&file)
            .unwrap()
            .file_type()
            .is_fifo(),
        "the FIFO stays"
    );
    std::fs::remove_file(&file).unwrap();

    saved::write_file(&file, &text).expect("with the node gone it writes");
    assert_eq!(read_bounded(&file), Some(held()));
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp."))
        .collect();
    assert!(leftovers.is_empty(), "no temp file is left: {leftovers:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
