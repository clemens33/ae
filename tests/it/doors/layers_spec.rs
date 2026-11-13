//! Independently authored R0 acceptance fixtures, frozen by layerguard-nav.
//! The fixture strings are parsed Rust source, not compiled product modules.
//! These tests exercise the guard's shared per-file entry point; the real-tree
//! guard must call that same entry point and enumerate the `source_view` product.

use super::layers::{violations, violations_with};

fn findings(file: &str, source: &str) -> Vec<String> {
    violations(file, source).unwrap_or_else(|why| panic!("{file}: {why}"))
}

fn rejects(file: &str, source: &str) {
    assert!(
        !findings(file, source).is_empty(),
        "an unlisted upward spelling escaped: {file}: {source}"
    );
}

fn accepts(file: &str, source: &str) {
    assert!(
        findings(file, source).is_empty(),
        "a permitted fixture was rejected: {file}: {source}"
    );
}

#[test]
fn r0_rejects_a_new_upward_import() {
    rejects("src/json.rs", "use crate::cli::Request;");
}

#[test]
fn r0_rejects_production_after_a_test_only_item() {
    rejects(
        "src/json.rs",
        "#[cfg(test)] mod tests {}\nuse crate::cli::Request;",
    );
}

#[test]
fn r0_rejects_production_after_a_test_only_function() {
    rejects(
        "src/json.rs",
        "#[cfg(test)] fn fixture() {}\nfn live() { crate::cli::Request::parse(&[]); }",
    );
}

#[test]
fn r0_expands_grouped_imports_including_nested_groups() {
    for source in [
        "use crate::{time, cli::Request};",
        "use crate::{time, cli::{Request}};",
        "use crate::cli::{self, Request};",
        "use crate::{cli::{Request as Renamed}};",
        "use crate::cli::*;",
    ] {
        rejects("src/json.rs", source);
    }
}

#[test]
fn r0_sees_type_and_expression_paths_without_a_use_item() {
    for source in [
        "fn live() { crate::cli::Request::parse(&[]); }",
        "fn live(_: &crate::cli::Request) {}",
    ] {
        rejects("src/json.rs", source);
    }
}

#[test]
fn r0_submodule_files_inherit_their_top_level_layer() {
    rejects("src/json/child.rs", "use crate::cli::Request;");
}

#[test]
fn r0_counts_root_item_spellings_as_layer_six() {
    rejects("src/json.rs", "fn live() { crate::help_text(); }");
    rejects("src/json.rs", "use crate::{time, help_text};");
    accepts("src/cli.rs", "fn live() { crate::help_text(); }");
}

#[test]
fn r0_ignores_test_items_comments_and_string_contents() {
    accepts(
        "src/json.rs",
        r#"
        // use crate::cli::Request;
        const TEXT: &str = "crate::cli::Request";
        #[cfg(test)] use crate::cli::Request;
        #[cfg(test)] mod tests { fn fixture(_: crate::cli::Request) {} }
        fn live() {}
        "#,
    );
}

#[test]
fn r0_accepts_sideways_and_downward_paths() {
    accepts("src/json.rs", "use crate::time;");
    accepts("src/cli.rs", "use crate::{json, config, console};");
}

#[test]
fn r0_keeps_an_existing_debt_spelling_but_rejects_a_new_one_on_that_edge() {
    let rows = [("src/sanitize.rs", "crate::config::is_agent_name", 1)];
    assert!(
        violations_with(
            "src/sanitize.rs",
            "fn live() { crate::config::is_agent_name(\"seat\"); }",
            &rows,
        )
        .unwrap_or_else(|why| panic!("{why}"))
        .is_empty()
    );
    let findings = violations_with(
        "src/sanitize.rs",
        "use crate::config::Profile; fn live() { crate::config::is_agent_name(\"seat\"); }",
        &rows,
    )
    .unwrap_or_else(|why| panic!("{why}"));
    assert!(!findings.is_empty());
}

#[test]
fn r0_keeps_permanent_typed_argv_handshakes_without_exempting_the_module_edge() {
    let rows = [
        ("src/transport.rs", "crate::lifecycle::DetachedArgv", 1),
        ("src/transport.rs", "crate::console::shell::ShellArgv", 1),
    ];
    let source =
        "fn live(_: &crate::lifecycle::DetachedArgv, _: &crate::console::shell::ShellArgv) {}";
    assert!(
        violations_with("src/transport.rs", source, &rows)
            .unwrap_or_else(|why| panic!("{why}"))
            .is_empty()
    );
    let findings = violations_with(
        "src/transport.rs",
        &format!("{source}\nuse crate::lifecycle::LayerguardNewUpwardSpelling;"),
        &rows,
    )
    .unwrap_or_else(|why| panic!("{why}"));
    assert!(!findings.is_empty());
}

#[test]
fn r0_requires_pins_to_shrink_and_rejects_growth() {
    let once = "fn live() { crate::config::is_agent_name(\"seat\"); }";
    let twice = "fn live() { crate::config::is_agent_name(\"seat\"); crate::config::is_agent_name(\"other\"); }";
    let row = [("src/sanitize.rs", "crate::config::is_agent_name", 1)];
    assert!(
        violations_with("src/sanitize.rs", once, &row)
            .unwrap_or_else(|why| panic!("{why}"))
            .is_empty()
    );
    for source in ["fn live() {}", twice] {
        assert!(
            !violations_with("src/sanitize.rs", source, &row)
                .unwrap_or_else(|why| panic!("{why}"))
                .is_empty()
        );
    }
    let row = [("src/sanitize.rs", "crate::config::is_agent_name", 2)];
    assert!(
        violations_with("src/sanitize.rs", twice, &row)
            .unwrap_or_else(|why| panic!("{why}"))
            .is_empty()
    );
}

#[test]
fn r0_allow_rows_are_scoped_to_their_file() {
    let once = "fn live() { crate::config::is_agent_name(\"seat\"); }";
    let foreign_row = [("src/other.rs", "crate::config::is_agent_name", 1)];
    assert!(
        !violations_with("src/sanitize.rs", once, &foreign_row)
            .unwrap_or_else(|why| panic!("{why}"))
            .is_empty()
    );
    let row = [("src/sanitize.rs", "crate::config::is_agent_name", 1)];
    assert!(
        violations_with("src/json.rs", "fn live() {}", &row)
            .unwrap_or_else(|why| panic!("{why}"))
            .is_empty()
    );
}

#[test]
fn r0_sees_written_paths_inside_macro_arguments() {
    rejects(
        "src/json.rs",
        r#"fn live() { println!("{:?}", crate::cli::Request::parse(&[])); }"#,
    );
}

#[test]
fn r0_does_not_confuse_crate_visibility_with_a_path() {
    accepts("src/json.rs", "pub(crate) fn live() {}");
    accepts("src/lib.rs", "use crate::cli::Request;");
}

#[test]
fn r0_refuses_a_source_module_without_a_reviewed_layer() {
    assert!(
        violations("src/layerguard_unassigned.rs", "fn live() {}").is_err(),
        "a newly added module must obtain a reviewed layer before the guard passes"
    );
}

#[test]
fn r0_refuses_unparseable_source_instead_of_scanning_nothing() {
    assert!(violations("src/json.rs", "fn broken(").is_err());
}
