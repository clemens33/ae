//! Frozen P2 oracle: brief P2 GO, plan r4 choice 2a; base d5fd66b4.
#![allow(
    clippy::disallowed_methods,
    reason = "acceptance fixtures read and write only their sandbox HOME and private socket"
)]

use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};

use super::{Cleanup, Rig, SnapshotEntry, meta_of, present, snapshot, tmux, value_of};

const NEW: &str = "2099.1.1";
const OLD: &str = "2026.1.1";
const ORPHAN: &str = "2026.2.2";
const HELPERS: &[&str] = &[
    "send",
    "relay",
    "ask",
    "review",
    "reply",
    "requests",
    "state",
    "mark-done",
    "say",
    "memo",
    "goal",
    "peek",
    "peak",
    "agents",
    "quota",
    "usage",
    "focus",
    "interrupt",
    "spawn",
    "retire",
    "relaunch",
    "_register-sid",
    "watchdog",
    "loop",
    "events-tail",
];

fn plant(rig: &Rig, instance: &str, name: &str) -> PathBuf {
    let root = if instance.is_empty() {
        rig.root()
    } else {
        rig.root().join("instances").join(instance)
    };
    let dir = root.join("sessions").join(name);
    fs::create_dir_all(&dir).expect("fixture session");
    fs::write(
        dir.join("meta"),
        format!(
            "meta_version=3\nschema=2\nmode=local\nsession={name}\nwork_dir=/w\n\
             seat.main=lead\nprofile.main=cl\nae_core={}\nae_core_version={OLD}\n\
             ae_version=0.0.1\nnote={instance}\n",
            rig.versions().join(OLD).join("ae-core").display()
        ),
    )
    .expect("fixture meta");
    dir
}

fn installed(rig: &Rig, dir: &Path, note: &str) {
    let meta = meta_of(dir);
    let core = rig.versions().join(NEW).join("ae-core");
    for (key, expected) in [
        ("meta_version", "4"),
        ("ae_core", core.to_str().expect("fixture path")),
        ("ae_core_version", NEW),
        ("ae_version", NEW),
        ("note", note),
    ] {
        assert_eq!(
            value_of(&meta, key).as_deref(),
            Some(expected),
            "{key}: {meta}"
        );
    }
    for helper in HELPERS {
        assert_eq!(
            fs::read_link(dir.join(helper)).ok(),
            Some(core.clone()),
            "{helper}"
        );
    }
    assert_eq!(fs::read_link(rig.link()).ok(), Some(core));
}

fn object(path: &Path) -> Vec<SnapshotEntry> {
    let kind = fs::symlink_metadata(path)
        .expect("fixture entry")
        .file_type();
    if kind.is_dir() {
        snapshot(path)
    } else if kind.is_symlink() {
        vec![SnapshotEntry::Symlink(
            PathBuf::new(),
            fs::read_link(path).expect("link"),
        )]
    } else {
        vec![SnapshotEntry::File(
            PathBuf::new(),
            fs::read(path).expect("file"),
        )]
    }
}

#[test]
fn publish_steps_same_named_session_in_default_and_two_instances() {
    let rig = Rig::new("p2-fleet");
    rig.plant_version(OLD);
    let orphan = rig.plant_version(ORPHAN);
    let dirs = [
        plant(&rig, "", "demo"),
        plant(&rig, "a", "demo"),
        plant(&rig, "b", "demo"),
    ];
    let (code, out, err) = rig.install(&rig.bundle(NEW));
    assert_eq!(code, Some(0), "{out}{err}");
    for (dir, name) in dirs.iter().zip(["", "a", "b"]) {
        installed(&rig, dir, name);
    }
    assert!(!present(&orphan), "unreferenced version survived: {out}");
    for name in ["a", "b"] {
        let root = rig.root().join("instances").join(name);
        assert!(!present(&root.join("versions")) && !present(&root.join(".local")));
    }
    assert!(
        out.find("instance a")
            .zip(out.find("instance b"))
            .is_some_and(|(a, b)| a < b),
        "{out}"
    );
}

#[test]
fn prune_unions_both_named_core_rows_with_the_shared_link() {
    let rig = Rig::new("p2-union");
    let path_pin = rig.plant_version(OLD);
    let row_pin = rig.plant_version(ORPHAN);
    let stale = rig.plant_version("2026.3.3");
    let published = rig.plant_version(NEW);
    let link_pin = rig.plant_version("2026.4.4");
    fs::create_dir_all(rig.link().parent().expect("bin")).expect("bin");
    symlink(link_pin.join("ae-core"), rig.link()).expect("command link");
    let dir = plant(&rig, "a", "demo");
    fs::write(
        dir.join("meta"),
        meta_of(&dir).replace(
            &format!("ae_core_version={OLD}"),
            &format!("ae_core_version={ORPHAN}"),
        ),
    )
    .expect("distinct row pin");
    let before = snapshot(&dir);
    let notes = ae::migrate::prune_versions(&rig.root(), &rig.link(), NEW);
    assert!(
        present(&path_pin) && present(&row_pin),
        "named references pruned: {notes:?}"
    );
    assert!(present(&published) && present(&link_pin));
    assert!(
        !present(&stale),
        "union kept an unrelated version: {notes:?}"
    );
    assert_eq!(
        snapshot(&dir),
        before,
        "prune must be read-only outside versions"
    );
}

#[allow(
    clippy::too_many_lines,
    reason = "one damage matrix shares its untouched-root and zero-prune assertions"
)]
fn damage(tag: &str) {
    let rig = Rig::new(&format!("p2-{tag}"));
    rig.plant_version(OLD);
    let orphan = rig.plant_version(ORPHAN);
    let default = plant(&rig, "", "demo");
    let bad_session = plant(&rig, "a", "omega");
    plant(&rig, "a", "alpha");
    let good = plant(&rig, "z", "demo");
    let instances = rig.root().join("instances");
    let bad = instances.join("a");
    let outside = rig.scratch.join("outside");
    fs::create_dir(&outside).expect("owned outside target");
    let mut witness = bad.clone();
    let mut denied = None;
    let mut report_name = "a";
    let container = matches!(tag, "container-link" | "container-file");
    match tag {
        "container-link" | "container-file" => {
            fs::rename(&instances, outside.join("roots")).expect("move owned roots");
            if tag == "container-link" {
                symlink(outside.join("roots"), &instances).expect("container link");
            } else {
                fs::write(&instances, "blocked").expect("container file");
            }
            witness.clone_from(&instances);
            report_name = "instances";
        }
        "root-link" | "root-file" => {
            fs::rename(&bad, outside.join("root")).expect("move owned root");
            if tag == "root-link" {
                symlink(outside.join("root"), &bad).expect("root link");
            } else {
                fs::write(&bad, "blocked").expect("root file");
            }
        }
        "bad-name" | "reserved" => {
            report_name = if tag == "reserved" { "dev" } else { "Work" };
            witness = instances.join(report_name);
            fs::rename(&bad, &witness).expect("invalid instance name");
        }
        "sessions-link" => {
            fs::rename(bad.join("sessions"), outside.join("sessions")).expect("move sessions");
            symlink(outside.join("sessions"), bad.join("sessions")).expect("sessions link");
            fs::write(outside.join("sessions/.rename.old.new.intent"), "broken\n")
                .expect("foreign carrier must never be classified");
        }
        "session-link" | "session-file" => {
            fs::rename(&bad_session, outside.join("session")).expect("move session");
            if tag == "session-link" {
                symlink(outside.join("session"), &bad_session).expect("session link");
            } else {
                fs::write(&bad_session, "blocked").expect("session file");
            }
        }
        "meta-link" | "meta-dangling" => {
            let meta = bad_session.join("meta");
            fs::rename(&meta, outside.join("meta")).expect("move fixture meta");
            let target = outside.join(if tag == "meta-link" {
                "meta"
            } else {
                "missing"
            });
            symlink(target, meta).expect("meta link");
        }
        "meta-dir" => {
            fs::remove_file(bad_session.join("meta")).expect("remove fixture meta");
            fs::create_dir(bad_session.join("meta")).expect("unreadable meta");
        }
        "meta-absent-version" => {
            fs::write(bad_session.join("meta"), "mode=local\nsession=omega\n")
                .expect("unplaceable");
        }
        "meta-malformed" => {
            fs::write(bad_session.join("meta"), "meta_version=broken\n").expect("malformed");
        }
        "meta-ahead" => {
            fs::write(bad_session.join("meta"), "meta_version=999999\n").expect("ahead");
        }
        "meta-denied" | "census-denied" => {
            denied = Some(if tag == "meta-denied" {
                bad_session.join("meta")
            } else {
                bad.join("sessions")
            });
        }
        _ => panic!("unknown fixture {tag}"),
    }
    let before = object(&witness);
    let outside_before = snapshot(&outside);
    if let Some(path) = &denied {
        fs::set_permissions(path, fs::Permissions::from_mode(0o000)).expect("deny fixture read");
    }
    let (code, out, err) = rig.install(&rig.bundle(NEW));
    if let Some(path) = &denied {
        let mode = if tag == "meta-denied" { 0o644 } else { 0o755 };
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("restore fixture read");
    }
    assert_eq!(
        code,
        Some(0),
        "damaged named root blocked default: {out}{err}"
    );
    assert!(present(&orphan), "damage authorised pruning: {tag}: {out}");
    assert_eq!(
        object(&witness),
        before,
        "preflight wrote damaged root {tag}"
    );
    assert_eq!(
        snapshot(&outside),
        outside_before,
        "followed {tag} outside named root"
    );
    installed(&rig, &default, "");
    if !container {
        installed(&rig, &good, "z");
    }
    assert!(
        (out.contains(&format!("instance {report_name}"))
            || out.contains(&format!("instance {report_name:?}"))
            || (container && out.contains("instances")))
            && out.contains("WARNING: no version was removed"),
        "unnamed damage/prune: {out}"
    );
    if tag == "sessions-link" {
        assert!(
            !out.contains("a rename is in progress"),
            "classified a carrier behind the sessions symlink: {out}"
        );
    }
    if matches!(
        tag,
        "container-link" | "root-link" | "sessions-link" | "session-link"
    ) {
        let path = match tag {
            "container-link" => instances,
            "root-link" => bad,
            "sessions-link" => bad.join("sessions"),
            _ => bad_session,
        };
        fs::remove_file(path).expect("unlink owned fixture before Rig cleanup");
    }
}

macro_rules! damage_specs {
    ($($name:ident => $tag:literal),+ $(,)?) => { $(
        #[test]
        fn $name() { damage($tag); }
    )+ };
}

damage_specs! {
    container_symlink => "container-link", container_non_directory => "container-file",
    named_root_symlink => "root-link", named_root_non_directory => "root-file",
    invalid_named_root => "bad-name", reserved_named_root => "reserved",
    named_sessions_symlink => "sessions-link", named_session_symlink => "session-link",
    named_session_non_directory => "session-file", named_meta_directory => "meta-dir",
    named_meta_symlink => "meta-link", named_meta_dangling_symlink => "meta-dangling",
    stopped_unplaceable_named_meta => "meta-absent-version", malformed_named_meta => "meta-malformed",
    ahead_named_meta => "meta-ahead", unreadable_named_meta => "meta-denied",
    unreadable_named_census => "census-denied",
}

#[test]
fn prune_rechecks_readable_unplaceable_named_meta_and_removes_nothing() {
    let rig = Rig::new("p2-prune-doubt");
    let orphan = rig.plant_version(ORPHAN);
    let dir = plant(&rig, "a", "demo");
    fs::write(dir.join("meta"), "meta_version=broken\n").expect("damage after migration");
    let notes = ae::migrate::prune_versions(&rig.root(), &rig.link(), NEW);
    assert!(
        present(&orphan),
        "strict placement was not repeated: {notes:?}"
    );
    assert!(
        (notes.join("\n").contains("instance a") || notes.join("\n").contains("instance \"a\""))
            && notes.join("\n").contains("WARNING:"),
        "{notes:?}"
    );
}

#[test]
fn partial_named_relink_does_not_block_default_and_same_version_repairs_every_root() {
    let rig = Rig::new("p2-repair");
    let old = rig.plant_version(OLD);
    let orphan = rig.plant_version(ORPHAN);
    let default = plant(&rig, "", "demo");
    let failed = plant(&rig, "a", "demo");
    let good = plant(&rig, "z", "demo");
    fs::create_dir(failed.join("send")).expect("helper obstruction");
    let bundle = rig.bundle(NEW);
    let (code, out, err) = rig.install(&bundle);
    assert_eq!(code, Some(0), "named partial failure aborted: {out}{err}");
    assert!(
        out.contains("PARTIALLY relinked") && out.contains("instance a"),
        "partial failure hidden: {out}"
    );
    assert!(
        !out.contains("ae command was NOT moved") && !out.contains("ae command was not moved"),
        "false rollback: {out}"
    );
    installed(&rig, &default, "");
    installed(&rig, &good, "z");
    assert!(
        present(&old) && present(&orphan),
        "pruned after partial failure: {out}"
    );
    fs::remove_dir(failed.join("send")).expect("repair obstruction");
    let added = plant(&rig, "b", "demo");
    let (code, out, err) = rig.install(&bundle);
    assert_eq!(code, Some(0), "same-version repair refused: {out}{err}");
    for (dir, name) in [(&default, ""), (&failed, "a"), (&added, "b"), (&good, "z")] {
        installed(&rig, dir, name);
    }
    assert!(
        !present(&old) && !present(&orphan),
        "repair did not resume pruning: {out}"
    );
}

#[test]
fn default_running_refusal_happens_before_any_named_repoint() {
    let rig = Rig::new("p2-default-first");
    let (code, out, err) = rig.install(&rig.bundle(OLD));
    assert_eq!(code, Some(0), "{out}{err}");
    let named = plant(&rig, "a", "demo");
    let before = snapshot(&named);
    let scratch = super::tmux_scratch("p2-first");
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let dir = rig.legacy_session("legacy");
    fs::write(
        dir.join("meta"),
        format!(
            "{}tmux_server_kind=socket\ntmux_server={}\n",
            meta_of(&dir),
            socket.display()
        ),
    )
    .expect("private server rows");
    assert!(
        tmux(
            &socket,
            &scratch,
            &["new-session", "-d", "-s", "legacy", "sleep", "60"]
        )
        .0
    );
    let (code, out, err) = rig.install(&rig.bundle(NEW));
    assert_eq!(code, Some(1), "default refusal changed: {out}{err}");
    assert_eq!(
        snapshot(&named),
        before,
        "named roots moved before default preflight"
    );
    assert_eq!(
        fs::read_link(rig.link()).ok(),
        Some(rig.versions().join(OLD).join("ae-core"))
    );
}

#[test]
fn no_instances_keeps_default_publish_bytes_and_dot_entries_allow_pruning() {
    for dot in [false, true] {
        let rig = Rig::new(if dot { "p2-dot" } else { "p2-absent" });
        rig.plant_version(OLD);
        let orphan = rig.plant_version(ORPHAN);
        let default = plant(&rig, "", "demo");
        if dot {
            fs::create_dir_all(rig.root().join("instances/empty")).expect("empty named root");
            fs::write(rig.root().join("instances/.DS_Store"), "ignored").expect("dot file");
        }
        let (code, out, err) = rig.install(&rig.bundle(NEW));
        assert_eq!(code, Some(0), "{out}{err}");
        installed(&rig, &default, "");
        assert!(!present(&orphan), "safe prune suppressed: {out}");
        assert_eq!(err, "");
        assert_eq!(
            out,
            format!(
                "ae: migrated 1 session(s) from meta_version=3 to 4\nae: removed the unreferenced version {OLD}\nae: removed the unreferenced version {ORPHAN}\nae: installed {NEW} under {}\nae {NEW} installed: ~/.local/bin/ae — if `ae` is not found, open a new shell or add ~/.local/bin to PATH\n",
                rig.versions().join(NEW).display()
            )
        );
    }
}
