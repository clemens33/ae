//! Frozen P7 acceptance cases, authored by layoutp7-rev (Anthropic).
//! Phase 7 of the source layout: the ratchet keys on folders. The module->layer map comes
//! from the module declarations of the seven layer files of the tree under check, and no
//! `src/*.rs` other than src/lib.rs, src/main.rs and the seven layer files is accepted.
//! Every case enters through `violations_in`, the entry point the real-tree guard uses.

use std::path::Path;

use super::layers::violations_in;
use super::layers_allow::{DEBT, HANDSHAKE, Row};
use crate::support::source_view;

// Build every `src/` path from pieces, so a planted path adds no gate data edge.
fn source(module: &str) -> String {
    const SOURCE: &str = "src";
    format!("{SOURCE}/{module}.rs")
}

/// Everything the guard says about `files`: the findings, or the reason it cannot place them.
fn report(files: &[(String, String)], allow: &[Row]) -> String {
    match violations_in(files, allow) {
        Ok(findings) => findings.join("\n"),
        Err(reason) => reason,
    }
}

fn fixture(files: &[(&str, &str)]) -> Vec<(String, String)> {
    files
        .iter()
        .map(|(module, text)| (source(module), (*text).to_owned()))
        .collect()
}

/// The pinned rows of the real tree, debt and handshakes.
fn real_rows() -> Vec<Row> {
    DEBT.iter()
        .copied()
        .chain(
            HANDSHAKE
                .iter()
                .map(|(file, text, count, _)| (*file, *text, *count)),
        )
        .collect()
}

/// The real `src/` tree, as the real-tree guard reads it.
fn real_tree() -> Vec<(String, String)> {
    source_view::tree()
        .iter()
        .filter(|(path, _)| path.starts_with("src/"))
        .cloned()
        .collect()
}

/// Append `extra` to the text of `module`'s file in `tree`, on a line of its own, and return
/// the 1-based line the first appended line lands on.
fn append(tree: &mut [(String, String)], module: &str, extra: &str) -> usize {
    let path = source(module);
    let Some((_, text)) = tree.iter_mut().find(|(file, _)| *file == path) else {
        panic!("{path} is not in the real tree");
    };
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let line = text.lines().count() + 1;
    text.push_str(extra);
    text.push('\n');
    line
}

/// The real tree with `module` declared in `layer` (`l2_model`), re-exported by src/lib.rs,
/// given a file, and named from the base layer's `src/l0_base/json.rs`.
fn planted_module(layer: &str, module: &str) -> Vec<(String, String)> {
    let mut tree = real_tree();
    append(&mut tree, layer, &format!("pub mod {module};"));
    append(&mut tree, "lib", &format!("pub use {layer}::{module};"));
    append(
        &mut tree,
        "l0_base/json",
        &format!("use crate::{module}::Marker;"),
    );
    tree.push((
        source(&format!("{layer}/{module}")),
        "pub struct Marker;\n".to_owned(),
    ));
    tree
}

#[test]
fn p7_control_the_real_tree_is_clean() {
    let tree = real_tree();
    assert_eq!(report(&tree, &real_rows()), "");
}

#[test]
fn p7_control_an_unregistered_src_new_rs_is_refused() {
    // Control, green before and after P7: the shared view refuses a file nothing reaches.
    let mut tree = real_tree();
    tree.push((source("new"), "pub fn planted() {}\n".to_owned()));
    let got = report(&tree, &real_rows());
    assert!(got.contains(&source("new")), "{got}");
}

#[test]
fn p7_a_registered_flat_module_is_refused_at_its_file_and_its_declaration() {
    let mut tree = real_tree();
    let line = append(&mut tree, "lib", "pub mod new;");
    tree.push((source("new"), "pub fn planted() {}\n".to_owned()));
    let got = report(&tree, &real_rows());
    let site = format!("{}:{line}", source("lib"));
    assert!(got.contains(&source("new")), "{got}");
    assert!(
        got.lines()
            .any(|finding| finding.contains(&site) && finding.contains("new")),
        "expected a finding at {site} naming `new`, got:\n{got}"
    );
}

#[test]
fn p7_control_a_planted_upward_import_in_json_is_base_to_terminal_ui() {
    // Diagnostic control: green before and after P7.
    let mut tree = real_tree();
    let line = append(
        &mut tree,
        "l0_base/json",
        "use crate::console::P7PlantedUpward;",
    );
    let got = report(&tree, &real_rows());
    let site = format!("{}:{line}", source("l0_base/json"));
    let hits: Vec<&str> = got.lines().collect();
    assert_eq!(hits.len(), 1, "{got}");
    assert!(hits[0].contains(&site), "{got}");
    assert!(
        hits[0].contains("`crate::console::P7PlantedUpward`"),
        "{got}"
    );
    assert!(hits[0].contains("base -> terminal UI"), "{got}");
}

#[test]
fn p7_a_module_declared_in_a_layer_file_takes_that_layer_without_a_table() {
    // The declaration alone gives the new module its layer: the only finding is the upward
    // name from the base layer, classified by the declaring layer file.
    for (layer, edge) in [
        ("l5_ui", "base -> terminal UI"),
        ("l2_model", "base -> state and model"),
        ("l6_shell", "base -> CLI shell"),
    ] {
        let tree = planted_module(layer, "p7_planted");
        let got = report(&tree, &real_rows());
        let hits: Vec<&str> = got.lines().collect();
        assert_eq!(hits.len(), 1, "{layer}: {got}");
        assert!(hits[0].contains(&source("l0_base/json")), "{layer}: {got}");
        assert!(
            hits[0].contains("`crate::p7_planted::Marker`"),
            "{layer}: {got}"
        );
        assert!(hits[0].contains(edge), "{layer}: want `{edge}`, got {got}");
    }
}

#[test]
fn p7_a_module_declared_in_the_base_layer_is_sideways_from_json() {
    let tree = planted_module("l0_base", "p7_planted");
    assert_eq!(report(&tree, &real_rows()), "");
}

#[test]
fn p7_a_name_declared_by_two_layer_files_is_refused() {
    // Without a table, two declarations of one name would give `crate::time` two layers.
    // The second declaration is refused whether it loads its own file or another one.
    for (declaration, loaded) in [
        ("pub mod time;", "l5_ui/time"),
        (
            r#"#[path = "l5_ui/console.rs"] pub mod time;"#,
            "l5_ui/console",
        ),
    ] {
        let tree = fixture(&[
            ("lib", "mod l0_base;\nmod l5_ui;\npub use l0_base::time;"),
            ("l0_base", "pub mod time;"),
            ("l0_base/time", ""),
            ("l5_ui", declaration),
            (loaded, ""),
        ]);
        let got = report(&tree, &[]);
        let base = format!("{}:1", source("l0_base"));
        let ui = format!("{}:1", source("l5_ui"));
        assert!(
            got.lines().any(|finding| finding.contains("time")
                && finding.contains(&base)
                && finding.contains(&ui)),
            "{declaration}: expected one finding naming `time`, {base} and {ui}, got:\n{got}"
        );
    }
}

#[test]
fn p7_control_a_fixture_of_roots_and_layer_files_is_accepted_and_a_stray_root_file_is_not() {
    // Control, green before and after P7.
    let clean = [
        (
            "lib",
            "mod l0_base;\nmod l6_shell;\npub use l0_base::json;\npub use l6_shell::cli;",
        ),
        ("main", "fn main() {}"),
        ("l0_base", "pub mod json;"),
        ("l0_base/json", ""),
        ("l6_shell", "pub mod cli;"),
        ("l6_shell/cli", "use crate::json;"),
    ];
    assert_eq!(report(&fixture(&clean), &[]), "");
    for stray in ["new", "l7_extra", "base", "mod"] {
        let mut tree = fixture(&clean);
        tree.push((source(stray), String::new()));
        let got = report(&tree, &[]);
        assert!(got.contains(&source(stray)), "{stray}: {got}");
    }
}

fn read(path: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
        .unwrap_or_else(|error| panic!("{path}: {error}"))
}

#[test]
fn p7_the_hand_table_is_gone_from_the_guard_and_the_contract() {
    for path in [
        "tests/it/doors/layers.rs",
        "tests/it/doors/layers_map.rs",
        "docs/reference/contract.md",
    ] {
        assert!(
            !read(path).contains("MODULE_LAYERS"),
            "{path} still names MODULE_LAYERS"
        );
    }
}

#[test]
fn p7_the_docs_state_the_folder_rule_the_locator_and_the_ruling() {
    let contract = read("docs/reference/contract.md");
    let Some((_, map)) = contract.split_once("### Layer map") else {
        panic!("docs/reference/contract.md has no `### Layer map`");
    };
    let map = map.split("\n#").next().unwrap_or(map).to_lowercase();
    assert!(
        map.contains("by its folder and the layer file"),
        "the Layer map rule must say a new module gets its layer by its folder and the layer \
         file's declaration"
    );
    assert!(
        read("AGENTS.md").contains("ls src/*/"),
        "AGENTS.md has no `ls src/*/` module locator"
    );
    let history = read("docs/history.md");
    let Some((_, thirteen)) = history.split_once("\n## 13.") else {
        panic!("docs/history.md has no section 13");
    };
    let thirteen = thirteen.split("\n## 14.").next().unwrap_or(thirteen);
    assert!(
        thirteen.contains("ae-20261118T134348Z-120bcdc3"),
        "docs/history.md section 13 does not cite ruling ae-20261118T134348Z-120bcdc3"
    );
    let lib = read(&source("lib"));
    let header: String = lib
        .lines()
        .take_while(|line| line.starts_with("//!"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !header.contains("beside a future"),
        "the src/lib.rs header still describes the flat layout: {header}"
    );
}
