//! Supplementary lifecycle checks for dangling names and prune/rename exclusion.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "unit fixtures inspect only their own scratch entries"
)]

use std::fs;
use std::path::PathBuf;

use super::{create_alias, prune, reclaim, resolve, target};

struct Scratch {
    path: PathBuf,
    _owner: crate::tool_stats_fixture::Fixture,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let owner = crate::tool_stats_fixture::Fixture::new();
        let root = owner.0.join(format!("regsid-maint-{tag}"));
        fs::create_dir_all(root.join("sessions")).expect("sessions root");
        Self {
            path: root,
            _owner: owner,
        }
    }

    fn dir(&self, name: &str) -> PathBuf {
        self.path.join("sessions").join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn parent_is_not_a_direct_child_and_never_reclaimed() {
    let root = Scratch::new("parent");
    let alias = root.dir("old");
    let parent = root.dir("..");
    std::os::unix::fs::symlink(&parent, &alias).expect("parent link");
    assert!(target(&alias).is_none(), "parent escapes sessions root");
    reclaim(&alias).expect("unrecognized entry is left alone");
    assert_eq!(fs::read_link(&alias).expect("kept link"), parent);
}

#[test]
fn dangling_alias_is_reclaimable_without_a_target() {
    let root = Scratch::new("reclaim");
    let alias = root.dir("old");
    let missing = root.dir("missing");
    create_alias(&alias, &missing).expect("dangling alias");
    reclaim(&alias).expect("dangling name reclaimed");
    assert!(fs::symlink_metadata(&alias).is_err());
}

#[test]
fn relative_session_roots_keep_working_filesystem_helpers() {
    let root = Scratch::new("relative");
    fs::create_dir(root.dir("new")).expect("new directory");
    let cwd = std::env::current_dir().expect("test cwd");
    let mut relative = PathBuf::new();
    for component in cwd.components() {
        if matches!(component, std::path::Component::Normal(_)) {
            relative.push("..");
        }
    }
    relative.push(root.path.strip_prefix("/").expect("absolute fixture root"));
    let alias = relative.join("sessions/old");
    let new = relative.join("sessions/new");
    create_alias(&alias, &new).expect("relative root alias");
    assert_eq!(
        fs::read_link(&alias).expect("relative root readlink"),
        std::path::Path::new("new"),
        "relative roots need a sibling link, without repeating the state path"
    );
    assert_eq!(resolve(&alias), Some(new));
}

#[test]
fn pruning_keeps_busy_and_live_targets_and_foreign_entries() {
    let root = Scratch::new("prune");
    let missing = root.dir("missing");
    let alias = root.dir("old");
    create_alias(&alias, &missing).expect("dangling alias");
    let live = root.dir("live");
    fs::create_dir(&live).expect("live target");
    create_alias(&root.dir("live-old"), &live).expect("live alias");
    fs::write(root.dir("file"), b"keep").expect("foreign file");
    std::os::unix::fs::symlink(&root.path, root.dir("external")).expect("external link");
    let held = crate::lifecycle::lock(&root.path, "missing").expect("rename target lock");
    prune(&root.path, None).expect("doctor skips busy rename target");
    assert!(fs::read_link(&alias).is_ok(), "busy alias stays");
    drop(held);
    prune(&root.path, None).expect("prune after rename lock released");
    assert!(
        fs::symlink_metadata(&alias).is_err(),
        "dangling alias pruned"
    );
    assert_eq!(
        fs::read_link(root.dir("live-old")).expect("live link"),
        live
    );
    assert_eq!(fs::read(root.dir("file")).expect("kept file"), b"keep");
    assert_eq!(
        fs::read_link(root.dir("external")).expect("external link"),
        root.path
    );
}
