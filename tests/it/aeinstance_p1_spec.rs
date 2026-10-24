//! P1 default contract, frozen independently against main 1b098f3a.
#![allow(
    clippy::disallowed_methods,
    reason = "spec fixtures and the source census read only their owned tree"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[test]
fn installed_default_paths_server_and_telegram_home_keep_their_spellings() {
    let core = format!("/spec-home/.ae/versions/{}/ae-core", ae::VERSION);
    let shape = ae::shape::classify(Path::new(&core), Some(Path::new("/spec-home")));
    let root = Path::new("/spec-home/.ae");
    assert_eq!(shape.published_home(), Some(root));
    assert_eq!(ae::doors::state_root(&shape).as_deref(), Some(root));
    assert_eq!(ae::doors::config_file(&shape, root), root.join("config"));
    assert_eq!(
        shape.command_link().as_deref(),
        Some(Path::new("/spec-home/.local/bin/ae"))
    );
    assert_eq!(
        ae::doors::launch_target(ae::doors::declared_server(&shape).as_ref()),
        Some(ae::inventory::ServerId::Selected(ae::meta::Selector::Name(
            "ae".to_owned()
        )))
    );
    for (state, home) in [
        ("/spec-home/.ae", "/spec-home"),
        ("/scratch/state", "/scratch"),
        ("/", "/"),
    ] {
        let paths = ae::telegram::bridge::Paths::under(state);
        assert_eq!(paths.ae_home, Path::new(state));
        assert_eq!(paths.home, Path::new(home));
        assert_eq!(paths.config, Path::new(state).join("config"));
        assert_eq!(paths.state(), Path::new(state).join("telegram"));
    }
}

#[test]
fn default_pane_argv_and_reply_footer_keep_their_bytes() {
    let scratch = super::cli::OwnedScratch::root("instance-p1", "pane");
    let home = scratch.path().join("home");
    let published = home.join(".ae/versions").join(ae::VERSION).join("ae-core");
    let link = home.join(".local/bin/ae");
    fs::create_dir_all(published.parent().expect("version dir")).expect("version dir");
    fs::create_dir_all(link.parent().expect("bin dir")).expect("bin dir");
    fs::write(&published, "").expect("fixture core");
    std::os::unix::fs::symlink(&published, &link).expect("public link");
    let installed = ae::shape::classify(&published, Some(&home));
    let core = Path::new("/checkout core/ae");
    let dir = Path::new("/state root/sessions/demo");
    assert_eq!(
        ae::run::pane_command_for(&installed, &published, dir, "main"),
        format!(
            "'{}' _run '/state root/sessions/demo' 'main'",
            link.display()
        )
    );
    assert_eq!(
        ae::run::pane_command_for(&ae::shape::Shape::Checkout, core, dir, "spawned.9"),
        "'/checkout core/ae' _run '/state root/sessions/demo' 'spawned.9'"
    );
    assert_eq!(
        ae::run::pane_command_with_snapshot_for(
            &ae::shape::Shape::Checkout,
            core,
            dir,
            "main",
            "env KEY=x tool --flag"
        ),
        "'/checkout core/ae' _run --command-snapshot 'env KEY=x tool --flag' '/state root/sessions/demo' 'main'"
    );
    for (session, installed, expected) in [
        (
            "demo",
            true,
            "ae @demo reply --as \"lead\" \"request-1\" \"answer\"",
        ),
        (
            "demo",
            false,
            "/state root/sessions/demo/reply --as \"lead\" \"request-1\" \"answer\"",
        ),
        (
            "retained.name",
            true,
            "/state root/sessions/demo/reply --as \"lead\" \"request-1\" \"answer\"",
        ),
    ] {
        assert_eq!(
            ae::tracked::reply_command_for(dir, session, "lead", "request-1", "answer", installed),
            expected
        );
    }
    let send = ae::shim::lookup("send").expect("contract helper");
    assert_eq!(
        ae::shim::translate(send, dir, &["lead".to_owned(), "hello".to_owned()]),
        ["_send", "/state root/sessions/demo", "lead", "hello"]
    );
}

// Conservative lexical inventory, not a data-flow proof. Includes test halves:
// transport has production after a cfg(test) module, so truncating there misses
// real producers. Comments alone cannot add or hide a site; changed/moved/new
// references require review, including additions in an unlisted source file.
const NEEDLES: [&str; 6] = [
    "resolved_exe()",
    ".command_link()",
    "DetachedArgv(",
    "run_detached(",
    "run_autoupgrade_detached",
    "spawn_detached(",
];
const CENSUS: [(&str, [usize; 6]); 21] = [
    ("src/archive/retained.rs", [0, 1, 0, 0, 0, 0]),
    ("src/autoreseat/leg.rs", [1, 0, 0, 0, 0, 1]),
    ("src/autoupgrade.rs", [0, 0, 2, 0, 2, 0]),
    ("src/console/toggle.rs", [1, 0, 0, 0, 0, 0]),
    ("src/doctor.rs", [3, 0, 0, 0, 0, 0]),
    ("src/effort/leg.rs", [1, 0, 0, 0, 0, 1]),
    ("src/lib.rs", [2, 0, 0, 0, 0, 0]),
    ("src/lifecycle.rs", [4, 0, 4, 4, 0, 0]),
    ("src/lifecycle/end.rs", [2, 0, 2, 1, 0, 0]),
    ("src/rename.rs", [2, 0, 0, 0, 0, 0]),
    ("src/run.rs", [0, 1, 0, 0, 0, 0]),
    ("src/seat_relaunch.rs", [1, 0, 0, 0, 0, 0]),
    ("src/session_launch.rs", [2, 0, 0, 0, 0, 0]),
    ("src/session_launch/capture.rs", [1, 0, 0, 0, 0, 1]),
    ("src/session_menu.rs", [2, 0, 0, 0, 0, 0]),
    ("src/session_tmux.rs", [0, 1, 0, 0, 0, 0]),
    ("src/shape.rs", [2, 1, 0, 0, 0, 0]),
    ("src/spawn.rs", [1, 0, 0, 0, 0, 0]),
    ("src/telegram_lifecycle.rs", [2, 0, 0, 0, 0, 0]),
    ("src/transport.rs", [0, 0, 0, 1, 1, 1]),
    ("src/watchdog_lifecycle.rs", [1, 0, 0, 0, 0, 0]),
];

fn references(code: &str) -> [usize; 6] {
    let code = code
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    NEEDLES.map(|needle| code.matches(needle).count())
}

fn census(
    root: &Path,
    dir: &Path,
    found: &mut BTreeMap<String, [usize; 6]>,
) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            census(root, &path, found)?;
        } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
            let counts = references(&fs::read_to_string(&path)?);
            if counts.iter().any(|count| *count != 0) {
                let relative = path.strip_prefix(root).map_err(std::io::Error::other)?;
                found.insert(relative.to_string_lossy().replace('\\', "/"), counts);
            }
        }
    }
    Ok(())
}

#[test]
fn every_self_invocation_reference_has_a_reviewed_census_row() {
    // Independent positive/negative controls before the real whole-tree walk.
    assert_eq!(
        references("fn probe() { crate::shape::resolved_exe(); }"),
        [1, 0, 0, 0, 0, 0]
    );
    assert_eq!(references("// crate::shape::resolved_exe();"), [0; 6]);
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = BTreeMap::new();
    census(root, &root.join("src"), &mut found).expect("every source file is readable");
    let expected = CENSUS
        .map(|(file, counts)| (file.to_owned(), counts))
        .into_iter()
        .collect();
    assert_eq!(
        found, expected,
        "self-invocation census changed: review selection carriage before updating the frozen spec"
    );
}
