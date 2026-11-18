//! Frozen P0 folder and ownership cases from plan-r5.
//! The real-tree guard and these input fixtures use the same entry point.

use super::layers::violations_in;

type Fixture = Vec<(&'static str, &'static str)>;

// A fixture's module name is also a real MODULE_LAYERS name where it needs a
// reviewed layer. Build the virtual paths without adding gate data edges.
fn source(module: &str) -> String {
    const SOURCE: &str = "src";
    format!("{SOURCE}/{module}.rs")
}

fn findings(files: &[(&str, &str)]) -> Vec<String> {
    let files: Vec<_> = files
        .iter()
        .map(|(module, text)| (source(module), (*text).to_owned()))
        .collect();
    violations_in(&files, &[]).unwrap_or_else(|reason| panic!("cannot place fixture: {reason}"))
}

fn accepts(files: &[(&str, &str)]) {
    let got = findings(files);
    assert!(got.is_empty(), "placement should pass: {got:#?}");
}

fn refuses(files: &[(&str, &str)], module: &str, line: usize, evidence: &[&str]) {
    let got = findings(files);
    let location = format!("{}:{line}", source(module));
    assert!(
        got.iter().any(|message| {
            message.contains(&location) && evidence.iter().all(|part| message.contains(part))
        }),
        "expected finding at {location} naming {evidence:?}, got {got:#?}"
    );
}

const FOLDERED_ROOT: &str = "mod l0_base;\nmod l5_ui;\npub use l0_base::json;\npub use l5_ui::app;";

fn foldered_json(json: &'static str) -> Fixture {
    vec![
        ("lib", FOLDERED_ROOT),
        ("l0_base", "pub mod json;"),
        ("l0_base/json", json),
        ("l5_ui", "pub mod app;"),
        ("l5_ui/app", ""),
    ]
}

#[test]
fn p0_rejects_a_layer_registration_that_points_outside_its_folder() {
    let files = [
        ("lib", FOLDERED_ROOT),
        ("l0_base", r#"#[path = "l5_ui/borrowed.rs"] pub mod json;"#),
        ("l5_ui", "pub mod app;"),
        ("l5_ui/app", ""),
        ("l5_ui/borrowed", ""),
    ];

    refuses(&files, "l0_base", 1, &[&source("l5_ui/borrowed")]);
}

#[test]
fn p0_rejects_a_nested_registration_that_points_outside_its_owner_folder() {
    let mut files = foldered_json(r#"#[path = "../l5_ui/borrowed.rs"] pub mod borrowed;"#);
    files.push(("l5_ui/borrowed", "pub fn leaked() {}"));

    refuses(&files, "l0_base/json", 1, &[&source("l5_ui/borrowed")]);
}

#[test]
fn p0_rejects_a_crossing_hidden_under_an_inline_path_attribute() {
    let mut files = foldered_json(r#"#[path = "../l5_ui"] pub mod nested { pub mod borrowed; }"#);
    files.push(("l5_ui/borrowed", "pub fn leaked() {}"));

    refuses(&files, "l0_base/json", 1, &[&source("l5_ui/borrowed")]);
}

#[test]
fn p0_rejects_two_flat_owners_of_one_production_file() {
    let files = [
        ("lib", "pub mod json;\npub mod app;"),
        ("json", r#"#[path = "time.rs"] pub mod shared;"#),
        ("app", r#"#[path = "time.rs"] pub mod shared;"#),
        ("time", "pub fn shared() {}"),
    ];
    let got = findings(&files);
    let path = source("time");
    let base = format!("{}:1", source("json"));
    let ui = format!("{}:1", source("app"));

    assert!(
        got.iter().any(|finding| {
            finding.contains(&path)
                && ((finding.contains(&base) && finding.contains("app"))
                    || (finding.contains(&ui) && finding.contains("json")))
        }),
        "expected a conflicting-owner finding naming both registrations of {path}, got {got:#?}"
    );
}

#[test]
fn p0_rejects_a_flat_owned_file_inside_a_layer_folder() {
    let files = [
        ("lib", "mod l0_base;\npub use l0_base::json;\npub mod app;"),
        ("l0_base", "pub mod json;"),
        ("l0_base/json", ""),
        (
            "app",
            r#"#[path = "l0_base/borrowed.rs"] pub mod borrowed;"#,
        ),
        ("l0_base/borrowed", ""),
    ];

    refuses(&files, "app", 1, &[&source("l0_base/borrowed")]);
}

#[test]
fn p0_control_a_gated_registration_can_cross_between_flat_layers() {
    // This control also passes on the P0 parent.
    accepts(&[
        ("lib", "pub mod app;\npub mod stats_store;"),
        ("app", "mod maintainer;"),
        (
            "app/maintainer",
            "#[cfg(test)]\n#[path = \"maintainer_s2_spec.rs\"] mod maintainer_s2_spec;",
        ),
        (
            "app/maintainer_s2_spec",
            r#"#[path = "../stats_store/s2_cases.rs"] mod s2_cases;"#,
        ),
        ("stats_store", ""),
        ("stats_store/s2_cases", "use crate::app;"),
    ]);
}

#[test]
fn p0_accepts_a_gated_registration_crossing_between_layer_folders() {
    accepts(&[
        (
            "lib",
            "mod l3_services;\nmod l5_ui;\npub use l3_services::stats_store;\npub use l5_ui::app;",
        ),
        ("l3_services", "pub mod stats_store;"),
        ("l3_services/stats_store", ""),
        ("l3_services/stats_store/s2_cases", "use crate::l5_ui::app;"),
        ("l5_ui", "pub mod app;"),
        ("l5_ui/app", "mod maintainer;"),
        (
            "l5_ui/app/maintainer",
            "#[cfg(test)]\n#[path = \"maintainer_s2_spec.rs\"] mod maintainer_s2_spec;",
        ),
        (
            "l5_ui/app/maintainer_s2_spec",
            r#"#[path = "../../l3_services/stats_store/s2_cases.rs"] mod s2_cases;"#,
        ),
    ]);
}

#[test]
fn p0_rejects_a_root_alias_without_a_registration_in_its_layer() {
    let files = [
        (
            "lib",
            "mod l0_base;\npub use l0_base::time;\npub use l0_base::json;",
        ),
        ("l0_base", "pub mod time;"),
        ("l0_base/time", ""),
    ];

    refuses(&files, "lib", 3, &["l0_base", "json"]);
}

#[test]
fn p0_rejects_a_registration_that_disagrees_with_module_layers() {
    let files = [
        ("lib", "mod l5_ui;\npub use l5_ui::json;"),
        ("l5_ui", "pub mod json;"),
        ("l5_ui/json", ""),
    ];

    // The registration resolves and the file is correctly placed for its
    // claimed owner. The reviewed table assigns json to the base layer.
    refuses(&files, "l5_ui", 1, &["json", "base"]);
}

#[test]
fn p0_rejects_a_flat_module_in_a_foldered_layer() {
    let files = [
        ("lib", "mod l0_base;\npub use l0_base::json;\npub mod time;"),
        ("l0_base", "pub mod json;"),
        ("l0_base/json", ""),
        ("time", ""),
    ];

    refuses(&files, "lib", 3, &["time", "l0_base"]);
}

#[test]
fn p0_rejects_a_direct_layer_spelling_in_an_ordinary_module() {
    let files = [
        ("lib", "mod l0_base;\npub use l0_base::json;\npub mod cli;"),
        ("l0_base", "pub mod json;"),
        ("l0_base/json", "pub struct Value;"),
        ("cli", "use crate::l0_base::json::Value;"),
    ];

    refuses(&files, "cli", 1, &["crate::l0_base::json"]);
}

#[test]
fn p0_accepts_a_layer_path_in_visibility_and_root_aliases() {
    accepts(&[
        ("lib", "mod l0_base;\npub use crate::l0_base::json;"),
        ("l0_base", "pub mod json;"),
        ("l0_base/json", "pub(in crate::l0_base) fn local() {}"),
    ]);
}

#[test]
fn p0_rejects_an_explicit_crate_visible_flat_module_declaration() {
    refuses(
        &[("lib", "pub(crate) mod json;"), ("json", "")],
        "lib",
        1,
        &["pub(crate)", "json"],
    );
}

#[test]
fn p0_rejects_an_unqualified_private_module_declaration_in_a_layer_file() {
    refuses(
        &[
            ("lib", "mod l0_base;\npub(crate) use l0_base::json;"),
            ("l0_base", "mod json;"),
            ("l0_base/json", ""),
        ],
        "l0_base",
        1,
        &["private", "json"],
    );
}

#[test]
fn p0_rejects_parent_visible_module_declaration_in_a_layer_file() {
    refuses(
        &[
            ("lib", "mod l0_base;\npub(crate) use l0_base::json;"),
            ("l0_base", "pub(super) mod json;"),
            ("l0_base/json", ""),
        ],
        "l0_base",
        1,
        &["pub(super)", "json"],
    );
}
