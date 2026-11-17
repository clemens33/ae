//! Independent #279 part 1 live FIFO acceptance. Wire as a child of
//! `app_copy_spec.rs`: `#[path = "foldertree_fifo_spec.rs"] mod foldertree_fifo_spec;`.
//! Reuses its owned live rig and the existing integration FIFO fixture door.
use super::Rig;

#[test]
fn a_folder_fifo_is_listed_without_opening_and_escape_remains_responsive() {
    let rig = Rig::new("foldertree-fifo", &["preview ~/tree"]);
    let folder = rig.root.join("tree");
    std::fs::create_dir(&folder).expect("owned preview folder");
    let canonical = std::fs::canonicalize(&folder).expect("canonical owned preview folder");
    crate::cli::mkfifo(&folder.join("NAMED_FIFO_ENTRY"));
    std::fs::write(folder.join("REGULAR_ENTRY.txt"), b"not opened").expect("owned file");
    let app = rig.open();
    rig.raw(&app, "]");
    rig.wait_screen(&app, "marked folder link", |screen| {
        super::has_marked_body(screen, "preview ~/tree")
    });
    rig.raw(&app, "r");
    let header = format!("Read · {}", folder.display());
    let canonical_header = format!("Read · {}", canonical.display());
    let shown = rig.wait_screen(&app, "FIFO listed without a blocking open", |screen| {
        (screen.contains(&header) || screen.contains(&canonical_header))
            && screen.contains("NAMED_FIFO_ENTRY")
            && screen.contains("REGULAR_ENTRY.txt")
    });
    // Both spellings identify the fixture HOME supplied by Rig::open,
    // including macOS /var -> /private/var, independently of the real HOME.
    assert!(
        shown.contains(&header) || shown.contains(&canonical_header),
        "the reader must use the scratch home: {shown}"
    );
    assert!(!shown.contains("not a regular file"), "{shown}");
    rig.key(&app, "Escape");
    rig.wait_screen(
        &app,
        "Escape responds after the special-file walk",
        |screen| !screen.contains("Read · ") && screen.contains("preview ~/tree"),
    );
}
