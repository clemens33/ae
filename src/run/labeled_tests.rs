//! A labeled spawned seat resolves on the session's recorded GLOBAL config
//! alone, and every lookup that reuses its identity pair does too; an
//! unlabeled spawned seat and a labeled main seat keep the layered pair.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the fixture owns its state dir, configs and stand-in executables"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

struct Fixture {
    base: PathBuf,
    dir: PathBuf,
    global: PathBuf,
    overlay: PathBuf,
    global_bin: PathBuf,
    overlay_bin: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn exe(path: &Path) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("a bin dir");
    std::fs::write(path, "#!/bin/sh\nexit 0\n").expect("a stand-in executable");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("exec bit");
}

/// A session whose seat `slot` records profile `p` and, when `client` is set,
/// that client label. The global config defines `carry` on `b/claude`; the
/// project overlay, when `overlay` holds a row, redefines what it says.
fn fixture(tag: &str, slot: &str, client: Option<&str>, overlay: &str) -> Fixture {
    let base = PathBuf::from(format!("/tmp/ae-lbl-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let dir = base.join("state");
    let project = base.join("project");
    std::fs::create_dir_all(&dir).expect("a state dir");
    std::fs::create_dir_all(project.join(".ae")).expect("a project dir");
    let default_bin = base.join("a/claude");
    let global_bin = base.join("b/claude");
    let overlay_bin = base.join("c/claude");
    for bin in [&default_bin, &global_bin, &overlay_bin] {
        exe(bin);
    }
    for store in ["store-a", "store-b"] {
        std::fs::create_dir_all(base.join(store)).expect("a store");
    }
    let store_b = std::fs::canonicalize(base.join("store-b")).expect("store-b");
    let global = base.join("config");
    std::fs::write(
        &global,
        format!(
            "[clients]\ndefault = {} config_home={}\ncarry = {} config_home={}\n\n\
             [profiles]\np = \"default --model chosen\"\n",
            default_bin.display(),
            base.join("store-a").display(),
            global_bin.display(),
            store_b.display(),
        ),
    )
    .expect("the global config");
    let overlay_path = project.join(".ae/config");
    if !overlay.is_empty() {
        std::fs::write(
            &overlay_path,
            overlay
                .replace("{overlay_bin}", &overlay_bin.display().to_string())
                .replace("{store_b}", &store_b.display().to_string()),
        )
        .expect("the project overlay");
    }
    let client_row = client.map_or_else(String::new, |label| {
        format!(
            "client.{slot}={label}\nconfig_home.{slot}={}\n",
            store_b.display()
        )
    });
    std::fs::write(
        crate::store::open(&dir).meta_path(),
        format!(
            "session=lbl\nwork_dir={p}\norigin={p}\nconfig={g}\nseat.{slot}=scout\n\
             profile.{slot}=p\nagent_bin.{slot}=claude\n{client_row}",
            p = project.display(),
            g = global.display(),
        ),
    )
    .expect("the meta");
    Fixture {
        base,
        dir,
        global,
        overlay: overlay_path,
        global_bin,
        overlay_bin,
    }
}

const SHADOW: &str = "[clients]\ncarry = {overlay_bin} config_home={store_b}\n";

fn command(fx: &Fixture, slot: &str) -> Result<String, String> {
    super::read_seat(&fx.dir, slot, None).map(|seat| seat.command.as_str().to_owned())
}

#[test]
fn a_labeled_spawned_seat_runs_the_global_client_and_drops_the_overlay_from_its_pair() {
    let fx = fixture("spawned", "spawned.0", Some("carry"), SHADOW);
    let seat = super::read_seat(&fx.dir, "spawned.0", None).expect("the global pair judges");
    let command = seat.command.as_str();
    assert!(
        command.contains(&fx.global_bin.display().to_string()),
        "{command}"
    );
    assert!(
        !command.contains(&fx.overlay_bin.display().to_string()),
        "{command}"
    );
    assert_eq!(seat.identity_global.as_deref(), Some(fx.global.as_path()));
    assert_eq!(
        seat.identity_local, None,
        "the follow and effort twins read global only"
    );
}

#[test]
fn a_labeled_spawned_seat_ignores_a_malformed_overlay() {
    let fx = fixture(
        "broken",
        "spawned.0",
        Some("carry"),
        "[clients]\nnot a row\n",
    );
    assert!(
        command(&fx, "spawned.0")
            .is_ok_and(|cmd| cmd.contains(&fx.global_bin.display().to_string()))
    );
}

#[test]
fn a_labeled_spawned_seat_whose_client_only_the_overlay_defines_refuses_by_name() {
    let fx = fixture(
        "ghost",
        "spawned.0",
        Some("ghost"),
        "[clients]\nghost = {overlay_bin}\n",
    );
    let why = command(&fx, "spawned.0").expect_err("an overlay never approves a client");
    assert!(why.contains("'ghost'"), "{why}");
    assert!(
        why.contains("restore the global [clients] row") && why.contains("retire the seat"),
        "{why}"
    );
    assert!(
        !why.starts_with("Error:"),
        "a clause for the caller's frame: {why}"
    );
}

#[test]
fn a_labeled_spawned_seat_whose_profile_the_global_config_lacks_refuses() {
    let fx = fixture(
        "noprof",
        "spawned.0",
        Some("carry"),
        "[profiles]\np = \"carry\"\n",
    );
    std::fs::write(
        &fx.global,
        std::fs::read_to_string(&fx.global)
            .expect("global")
            .replace("p = \"default --model chosen\"\n", ""),
    )
    .expect("drop the global profile");
    let why = command(&fx, "spawned.0").expect_err("only a resume keeps a seat unstarted");
    assert!(why.contains("no profile 'p'"), "{why}");
}

#[test]
fn a_labeled_spawned_seat_with_no_global_config_refuses() {
    let fx = fixture("noglobal", "spawned.0", Some("carry"), SHADOW);
    std::fs::remove_file(&fx.global).expect("remove the global config");
    let why = command(&fx, "spawned.0").expect_err("no authority, no pair");
    assert!(why.contains("cannot be used"), "{why}");
}

#[test]
fn an_unlabeled_spawned_seat_keeps_the_layered_pair() {
    let fx = fixture(
        "bare",
        "spawned.0",
        None,
        "[clients]\ndefault = {overlay_bin} config_home={store_b}\n",
    );
    let seat = super::read_seat(&fx.dir, "spawned.0", None).expect("resolves");
    assert_eq!(seat.identity_local.as_deref(), Some(fx.overlay.as_path()));
    assert!(
        seat.command
            .as_str()
            .contains(&fx.overlay_bin.display().to_string()),
        "the overlay still layers over an unlabeled seat: {}",
        seat.command.as_str()
    );
}

#[test]
fn a_labeled_main_seat_keeps_the_layered_pair() {
    let fx = fixture("main", "main", Some("carry"), SHADOW);
    let seat = super::read_seat(&fx.dir, "main", None).expect("resolves");
    assert_eq!(seat.identity_local.as_deref(), Some(fx.overlay.as_path()));
    assert!(
        seat.command
            .as_str()
            .contains(&fx.overlay_bin.display().to_string()),
        "a human-typed --lead x@y keeps the layered resolution: {}",
        seat.command.as_str()
    );
}

#[test]
fn a_command_snapshot_still_wins_for_a_labeled_spawned_seat() {
    let fx = fixture("snap", "spawned.0", Some("carry"), SHADOW);
    let seat = super::read_seat(&fx.dir, "spawned.0", Some("claude --model x")).expect("snapshot");
    assert_eq!(seat.command.as_str(), "claude --model x");
    assert_eq!(seat.identity_local, None);
}
