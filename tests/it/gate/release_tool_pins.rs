use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use super::{read, recipe_text, root, run_in, run_in_env};
use crate::cli::OwnedScratch;

fn write_executable(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap_or_else(|why| panic!("{}: {why}", path.display()));
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .unwrap_or_else(|why| panic!("{}: {why}", path.display()));
}

fn version_pin<'a>(justfile: &'a str, name: &str) -> &'a str {
    justfile
        .lines()
        .filter_map(|line| line.split_once(" := "))
        .find_map(|(key, value)| (key == name).then_some(value))
        .and_then(|value| value.strip_prefix('"')?.strip_suffix('"'))
        .unwrap_or_else(|| panic!("the justfile pins {name}"))
}

fn declared_pins(justfile: &str) -> BTreeSet<String> {
    justfile
        .lines()
        .filter(|line| !line.starts_with([' ', '\t', '#']))
        .filter_map(|line| line.split_once(":=")?.0.split_whitespace().last())
        .filter(|key| key.ends_with("_VERSION"))
        .map(str::to_owned)
        .collect()
}

fn classified_pins(justfile: &str) -> BTreeSet<String> {
    let release = recipe_text(justfile, "release:");
    let mut classified: BTreeSet<String> = release
        .iter()
        .take_while(|line| *line != "disk_floor release")
        .filter(|line| line.starts_with("release_tool_pin "))
        .filter_map(|line| {
            let (_, rest) = line.split_once("{{")?;
            Some(rest.split_once("}}")?.0.trim().to_owned())
        })
        .collect();
    // These pins are for separate development lanes or the Linux bootstrap download.
    classified.extend(
        [
            "CARGO_FUZZ_VERSION",
            "LLVM_COV_VERSION",
            "MUTANTS_VERSION",
            "RUSTUP_VERSION",
            "VET_VERSION",
        ]
        .map(str::to_owned),
    );
    classified
}

#[test]
fn every_justfile_version_pin_has_a_release_decision() {
    let justfile = read(&root().join("justfile"));
    let classified = classified_pins(&justfile);
    assert_eq!(declared_pins(&justfile), classified);

    let extra = format!("{justfile}\nFUTURE_VERSION := \"1.0.0\"\n");
    assert_eq!(
        declared_pins(&extra)
            .difference(&classified_pins(&extra))
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["FUTURE_VERSION"],
    );
}

fn assert_preflight_refusal(
    checkout: &Path,
    (code, out, err): (i32, String, String),
    expected: &[&str],
) {
    assert_ne!(code, 0, "the release must refuse: {out}{err}");
    assert!(
        !checkout.join("disk-floor-reached").exists(),
        "the mismatch must stop release before the disk floor: {out}{err}"
    );
    for word in expected {
        assert!(err.contains(word), "missing {word:?}: {out}{err}");
    }
}

// RUN_START looks up HEAD. The disk floor's distinct toplevel lookup marks
// the boundary and stops release before it can reach any other step.
const GIT_DISK_FLOOR_SENTINEL: &str = r#"#!/bin/sh
if [ "$*" = "rev-parse --show-toplevel" ]; then
    touch disk-floor-reached
fi
exit 1
"#;

const CARGO_VERSION_FORWARDER: &str = r#"#!/bin/sh
case "$*" in
    'nextest --version'|'deny --version') exec "cargo-$1" --version ;;
    *) exit 90 ;;
esac
"#;

#[test]
fn release_refuses_tool_mismatches_before_disk_floor() {
    let scratch = OwnedScratch::root("gate", "rel-pins");
    let bin = scratch.join("bin");
    let checkout = scratch.join("checkout");
    for dir in [&bin, &checkout, &scratch.join("io")] {
        std::fs::create_dir(dir).unwrap_or_else(|why| panic!("{}: {why}", dir.display()));
    }
    let justfile = read(&root().join("justfile"));
    std::fs::write(checkout.join("justfile"), &justfile).expect("the actual release recipe");
    write_executable(&bin.join("git"), GIT_DISK_FLOOR_SENTINEL);
    write_executable(&bin.join("cargo"), CARGO_VERSION_FORWARDER);
    let set_version = |tool: &str, version: &str| {
        write_executable(
            &bin.join(tool),
            &format!(
                "#!/bin/sh\n[ \"$*\" = --version ] || exit 90\nprintf '%s\\n' '{tool} {version}'\n"
            ),
        );
    };
    for (tool, pin) in [
        ("cargo-nextest", "NEXTEST_VERSION"),
        ("cargo-deny", "DENY_VERSION"),
        ("taplo", "TAPLO_VERSION"),
    ] {
        set_version(tool, version_pin(&justfile, pin));
    }
    let (code, real_just, err) = run_in(&scratch, &checkout, "bash", &["-c", "type -P just"]);
    assert_eq!(code, 0, "the fixture needs just: {err}");
    let real_just = real_just.trim();
    let (code, just_version, err) = run_in(&scratch, &checkout, real_just, &["--version"]);
    assert_eq!(code, 0, "just must report its version: {err}");
    let just_version = just_version
        .trim()
        .strip_prefix("just ")
        .expect("just reports its version");
    // Keep the fixture independent of the test runner's just installation.
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").expect("the fixture needs PATH"),
    );
    let release = |just_pin: &str| {
        let fixture_pin = format!("JUST_VERSION={just_pin}");
        run_in_env(
            &scratch,
            &checkout,
            real_just,
            &[&fixture_pin, "release"],
            &[("PATH", &path), ("AE_LANE_MIN_FREE_GIB", "20")],
        )
    };
    let cliff_pin = version_pin(&justfile, "GIT_CLIFF_VERSION");
    let wrong_version = format!("{cliff_pin}0");
    set_version("git-cliff", &wrong_version);
    assert_preflight_refusal(
        &checkout,
        release(just_version),
        &[
            &bin.join("git-cliff").display().to_string(),
            &wrong_version,
            &format!("want {cliff_pin}"),
            "just rust-setup",
        ],
    );
    set_version("git-cliff", cliff_pin);

    let wrong_just = format!("{just_version}0");
    assert_preflight_refusal(
        &checkout,
        release(&wrong_just),
        &[
            &format!("found just {just_version} at "),
            &format!("Install just {wrong_just}"),
        ],
    );
    set_version("just", &wrong_just);
    assert_preflight_refusal(
        &checkout,
        release(just_version),
        &[
            &bin.join("just").display().to_string(),
            &format!("found just {wrong_just} at "),
            &format!("Install just {just_version}"),
        ],
    );
    assert_eq!(read(&checkout.join("justfile")), justfile);
    assert_eq!(
        std::fs::read_dir(&checkout).expect("list checkout").count(),
        1,
        "refusals leave the checkout with only its justfile",
    );

    set_version("just", just_version);
    let (code, out, err) = release(just_version);
    assert_eq!(code, 1, "the fixture stops at the disk floor: {out}{err}");
    assert!(
        checkout.join("disk-floor-reached").is_file(),
        "matching tools must reach the disk floor: {out}{err}"
    );
}
