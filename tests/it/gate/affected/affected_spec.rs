//! Independent M1 acceptance. Authority: lead ruling-p2, plan-p2.r2 and addendum 1.
//! These fixtures pin the accepted one-hop gap as well as positive dependencies.
#![allow(
    clippy::expect_used,
    reason = "independent fixture preconditions must fail explicitly"
)]

use std::collections::BTreeSet;
use std::fmt::Write as _;

use super::super::modtree::Files;
use super::{Change, Input, Mode, Reason, Selection, Status, Verdict, Why, decode, render, select};

fn files(pairs: &[(&str, &str)]) -> Files {
    pairs
        .iter()
        .map(|(p, s)| ((*p).to_owned(), (*s).to_owned()))
        .collect()
}

fn fixture() -> Input {
    let new = files(&[
        ("src/lib.rs", "pub mod changed; pub mod indirect;"),
        ("src/main.rs", "fn main() {}"),
        ("src/changed.rs", "pub fn value() -> u8 { 1 }"),
        (
            "src/indirect.rs",
            "pub fn value() -> u8 { crate::changed::value() }",
        ),
        (
            "tests/it/main.rs",
            "mod doors; mod floor; mod gate; mod direct; mod indirect; mod blackbox; mod unrelated;",
        ),
        ("tests/it/doors.rs", "#[test] fn door_guard() {}"),
        ("tests/it/floor.rs", "#[test] fn floor_guard() {}"),
        ("tests/it/gate.rs", "#[test] fn gate_guard() {}"),
        (
            "tests/it/direct.rs",
            "#[test] fn direct_call() { let _ = ae::changed::value(); }",
        ),
        (
            "tests/it/indirect.rs",
            "#[test] fn indirect_call() { let _ = ae::indirect::value(); }",
        ),
        (
            "tests/it/blackbox.rs",
            "#[test] fn binary_only() { let _ = std::process::Command::new(\"ae\"); }",
        ),
        ("tests/it/unrelated.rs", "#[test] fn unrelated_test() {}"),
    ]);
    Input {
        mode: Mode::Gate,
        base: "main".to_owned(),
        merge_base: "a".repeat(40),
        head: "b".repeat(40),
        tracked: new.keys().cloned().collect(),
        new,
        old: files(&[("src/changed.rs", "pub fn value() -> u8 { 0 }")]),
        changes: vec![Change {
            status: Status::Modified,
            path: "src/changed.rs".to_owned(),
        }],
    }
}

fn put(input: &mut Input, path: &str, text: &str) {
    input.tracked.insert(path.to_owned());
    input.new.insert(path.to_owned(), text.to_owned());
}

fn add_it(input: &mut Input, name: &str, text: &str) {
    write!(
        input.new.get_mut("tests/it/main.rs").expect("it root"),
        " mod {name};"
    )
    .expect("append the module declaration");
    put(input, &format!("tests/it/{name}.rs"), text);
}

fn only_edit(input: &mut Input, path: &str, before: &str, after: &str) {
    input.changes = vec![Change {
        status: Status::Modified,
        path: path.to_owned(),
    }];
    input.old.clear();
    if path.strip_suffix(".rs").is_some()
        && (path.starts_with("src/") || path.starts_with("tests/"))
    {
        input.old = files(&[(path, before)]);
        put(input, path, after);
    } else {
        input.tracked.insert(path.to_owned());
    }
}

fn subset(selection: &Selection) {
    assert!(
        matches!(selection.verdict, Verdict::Subset),
        "expected a subset; whole would conceal the M1 gap"
    );
    assert!(selection.reasons.is_empty(), "subset has whole reasons");
    assert!(selection.filterset.contains("kind(lib)"));
    assert!(selection.filterset.contains("kind(bin)"));
    for guard in ["doors", "floor", "gate"] {
        assert!(
            selection.owners.contains(guard),
            "missing mandatory {guard}"
        );
    }
}

fn whole(selection: &Selection, reason: &str) {
    assert!(
        matches!(selection.verdict, Verdict::Whole),
        "expected whole: {reason}"
    );
    assert!(
        selection.reasons.iter().any(|r| r.id == reason),
        "missing reason {reason}"
    );
}

// Frozen P0 cases from plan-r5. The keys describe a fixture tree assembled in
// memory, so they are built here without naming repository data dependencies.
fn p0_source(module: &str) -> String {
    const SOURCE: &str = "src";
    format!("{SOURCE}/{module}.rs")
}

fn p0_layer_alias_fixture() -> Input {
    let mut input = fixture();
    put(
        &mut input,
        &p0_source("lib"),
        "
        pub mod changed;
        pub mod indirect;
        mod l0_base;
        mod l3_services;
        mod l5_ui;
        pub use crate::l0_base::{error, json};
        pub(crate) use l3_services::fleet_read;
        pub use l5_ui::app;
        pub use error::{Error, Result};
        pub use fleet_read::current_world;
        ",
    );
    for (module, source) in [
        ("l0_base", "pub mod error; pub mod json;"),
        (
            "l0_base/error",
            "pub struct Error; pub type Result<T> = std::result::Result<T, Error>;",
        ),
        ("l0_base/json", "pub struct Json;"),
        ("l3_services", "pub(crate) mod fleet_read;"),
        ("l3_services/fleet_read", "pub fn current_world() {}"),
        ("l5_ui", "pub mod app;"),
        (
            "l5_ui/app",
            "pub mod draw; pub use draw::X; pub fn other() {}",
        ),
        ("l5_ui/app/draw", "pub struct X;"),
    ] {
        put(&mut input, &p0_source(module), source);
    }
    for (owner, source) in [
        (
            "p0_root_item",
            "#[test] fn consumer() { let _ = ae::Error; }",
        ),
        (
            "p0_private_item",
            "#[test] fn consumer() { ae::current_world(); }",
        ),
        (
            "p0_nested_namespace",
            "#[test] fn consumer() { let _ = ae::app::draw::X; }",
        ),
        (
            "p0_parent_item",
            "#[test] fn consumer() { let _ = ae::app::X; }",
        ),
        (
            "p0_app_other",
            "#[test] fn consumer() { ae::app::other(); }",
        ),
    ] {
        add_it(&mut input, owner, source);
    }
    input
}

fn p0_edit(input: &mut Input, module: &str) {
    let path = p0_source(module);
    let source = input.new[&path].clone();
    only_edit(
        input,
        &path,
        &source,
        &format!("{source}\nconst CHANGED: u8 = 1;"),
    );
}

fn p0_selection(selection: &Selection, fixtures: &[&str]) {
    subset(selection);
    let expected: BTreeSet<String> = ["doors", "floor", "gate"]
        .into_iter()
        .chain(fixtures.iter().copied())
        .map(str::to_owned)
        .collect();
    assert_eq!(selection.owners, expected);
}

#[test]
fn p0_a_root_item_reexport_resolves_through_a_grouped_module_alias() {
    let mut input = p0_layer_alias_fixture();
    p0_edit(&mut input, "l0_base/error");

    p0_selection(&select(&input), &["p0_root_item"]);
}

#[test]
fn p0_a_public_item_reexport_resolves_through_a_crate_visible_module_alias() {
    let mut input = p0_layer_alias_fixture();
    p0_edit(&mut input, "l3_services/fleet_read");

    p0_selection(&select(&input), &["p0_private_item"]);
}

#[test]
fn p0_a_nested_module_keeps_direct_and_parent_reexport_consumers() {
    let mut input = p0_layer_alias_fixture();
    p0_edit(&mut input, "l5_ui/app/draw");

    // The separate owner naming app::other must stay out of the subset.
    p0_selection(&select(&input), &["p0_nested_namespace", "p0_parent_item"]);
}

#[test]
fn p0_control_data_readers_keep_the_basename_and_full_path_rules() {
    // This control also passes on the P0 parent.
    let mut input = fixture();
    put(
        &mut input,
        &p0_source("lib"),
        "pub mod changed; pub mod indirect; mod l0_base;",
    );
    put(&mut input, &p0_source("l0_base"), "pub mod x;");
    put(&mut input, &p0_source("l0_base/x"), "pub fn target() {}");
    for (owner, source) in [
        (
            "p0_e2",
            r#"#[test] fn reader() { let _ = root().join("src").join("l0_base").join("x.rs"); }"#,
        ),
        (
            "p0_full",
            r#"#[test] fn reader() { let _ = root().join("src/l0_base/x.rs"); }"#,
        ),
        (
            "p0_short",
            r#"#[test] fn reader() { let _ = root().join("src").join("l0_base/x.rs"); }"#,
        ),
    ] {
        add_it(&mut input, owner, source);
    }
    p0_edit(&mut input, "l0_base/x");

    let got = select(&input);
    p0_selection(&got, &["p0_e2", "p0_full"]);
    let target = p0_source("l0_base/x");
    for (owner, rule) in [("p0_e2", "basename"), ("p0_full", "literal")] {
        assert!(
            got.why
                .iter()
                .any(|pick| pick.owner == owner && pick.rule == rule && pick.target == target),
            "{owner} must be selected because it names the changed file as {rule}"
        );
    }
}

#[test]
fn p0_a_visibility_edit_in_a_registered_layer_file_runs_the_whole_suite() {
    let mut input = p0_layer_alias_fixture();
    let layer = p0_source("l3_services");
    only_edit(
        &mut input,
        &layer,
        "pub(crate) mod fleet_read;",
        "pub mod fleet_read;",
    );

    let got = select(&input);
    whole(&got, "crate-root");
    assert!(
        got.reasons
            .iter()
            .any(|reason| reason.id == "crate-root" && reason.detail == layer),
        "the whole verdict must name the edited layer file"
    );
}

#[test]
fn p0_control_a_layer_file_can_register_only_a_newly_added_module_in_subset_mode() {
    // This control also passes on the P0 parent.
    let mut input = p0_layer_alias_fixture();
    let layer = p0_source("l3_services");
    let added = p0_source("l3_services/p0_fresh");
    only_edit(
        &mut input,
        &layer,
        "pub(crate) mod fleet_read;",
        "pub(crate) mod fleet_read; pub(crate) mod p0_fresh;",
    );
    put(&mut input, &added, "pub(crate) fn fresh() {}");
    input.changes.push(Change {
        status: Status::Added,
        path: added.clone(),
    });
    add_it(
        &mut input,
        "p0_fresh_reader",
        &format!(r#"#[test] fn reader() {{ let _ = std::fs::read("{added}"); }}"#),
    );

    let got = select(&input);
    p0_selection(&got, &["p0_fresh_reader"]);
    assert!(
        got.why.iter().any(|pick| {
            pick.owner == "p0_fresh_reader" && pick.rule == "literal" && pick.target == added
        }),
        "the subset must include the reader of the added file"
    );
}

#[test]
fn direct_source_selects_only_direct_consumers_units_and_guards() {
    let got = select(&fixture());
    subset(&got);
    assert_eq!(
        got.owners,
        BTreeSet::from([
            "direct".to_owned(),
            "doors".to_owned(),
            "floor".to_owned(),
            "gate".to_owned()
        ])
    );
    assert_eq!(
        got.filterset,
        "kind(lib) | kind(bin) | (binary_id(ae::it) & test(/^(direct|doors|floor|gate)::/))"
    );
    assert_eq!((got.not_selected, got.total), (3, 7));
    assert!(got.doctests);
    assert!(!got.check);
    assert!(
        got.why
            .iter()
            .any(|w| w.owner == "direct" && w.file == "tests/it/direct.rs" && w.line > 0)
    );
    assert!(
        !got.owners.contains("indirect"),
        "the accepted A -> B -> C gap must remain explicit"
    );
    assert!(
        !got.owners.contains("blackbox"),
        "a binary invocation alone does not name a source module"
    );
}

#[test]
fn use_trees_aliases_and_reexports_are_direct_consumers() {
    let mut input = fixture();
    put(
        &mut input,
        "src/lib.rs",
        "pub mod changed; pub mod indirect; pub use changed::value as public_value;",
    );
    add_it(
        &mut input,
        "uses",
        "use ae::{changed::{self as module, value as renamed}, indirect}; #[test] fn uses() { let _ = renamed(); }",
    );
    add_it(
        &mut input,
        "exported",
        "use ae::public_value; #[test] fn exported() { let _ = public_value(); }",
    );
    let got = select(&input);
    subset(&got);
    assert!(got.owners.contains("uses"));
    assert!(got.owners.contains("exported"));
    assert!(!got.owners.contains("indirect"));
}

#[test]
fn source_as_text_is_a_dependency_without_a_namespace_reference() {
    let mut input = fixture();
    add_it(
        &mut input,
        "quota_muse_spec",
        "#[test] fn quota_contract() { let _ = include_str!(\"../../src/changed.rs\"); }",
    );
    let got = select(&input);
    subset(&got);
    assert!(
        got.owners.contains("quota_muse_spec"),
        "lead B1: changed Rust is also data"
    );
    assert!(
        got.owners.contains("direct"),
        "data and namespace rules form a union"
    );
    assert!(
        got.why
            .iter()
            .any(|w| w.owner == "quota_muse_spec" && w.target.contains("changed.rs"))
    );
}

#[test]
fn source_view_product_search_selects_the_text_reader() {
    let mut input = fixture();
    add_it(
        &mut input,
        "source_search",
        "use super::support::source_view;
         #[test] fn finds_body() {
             assert!(source_view::product().files.iter().any(|file| file.text.contains(\"fn value(\")));
         }",
    );
    let got = select(&input);
    subset(&got);
    assert!(got.owners.contains("source_search"));
    assert!(got.filterset.contains("source_search"));
    assert!(!got.owners.contains("unrelated"));

    only_edit(&mut input, "docs/note.md", "before", "after");
    let got = select(&input);
    subset(&got);
    assert!(!got.owners.contains("source_search"));
}

#[test]
fn a_manifest_src_walk_selects_its_guard_for_each_source_edit() {
    let mut input = fixture();
    add_it(
        &mut input,
        "tree_guard",
        "#[test] fn visits_sources() {
             let root = std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(\"src\");
             let mut stack = vec![root];
             while let Some(dir) = stack.pop() {
                 for entry in std::fs::read_dir(dir).unwrap().flatten() {
                     let path = entry.path();
                     if path.is_dir() { stack.push(path); }
                     else { let _ = std::fs::read(path).unwrap(); }
                 }
             }
         }",
    );
    for path in ["src/changed.rs", "src/indirect.rs"] {
        only_edit(
            &mut input,
            path,
            "pub fn value() {}",
            "pub fn value() { () }",
        );
        let got = select(&input);
        subset(&got);
        assert!(
            got.filterset.contains("tree_guard"),
            "guard omitted for {path}"
        );
        assert!(!got.owners.contains("unrelated"));
    }
    only_edit(&mut input, "src/render/template.txt", "before", "after");
    let got = select(&input);
    subset(&got);
    assert!(got.filterset.contains("tree_guard"));

    only_edit(&mut input, "docs/note.md", "before", "after");
    let got = select(&input);
    subset(&got);
    assert!(!got.owners.contains("tree_guard"));
}

#[test]
fn source_views_with_tests_select_their_owners_for_a_test_source_edit() {
    let mut input = fixture();
    for (name, reader) in [
        (
            "tree_reader",
            "use super::support::source_view::{tree as files};
             #[test] fn reads_tests() {
                 let _ = files().iter().filter(|(path, _)| path.starts_with(\"tests/it/\")).count();
             }",
        ),
        (
            "both_roots",
            "#[test] fn reads_roots() {
                 let root = std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\"));
                 for dir in [root.join(\"src\"), root.join(\"tests\")] {
                     let _ = std::fs::read_dir(dir).unwrap().count();
                 }
             }",
        ),
        (
            "product_only",
            "#[test] fn reads_product() { let _ = crate::support::source_view::product(); }",
        ),
    ] {
        add_it(&mut input, name, reader);
    }
    only_edit(
        &mut input,
        "tests/it/unrelated.rs",
        "#[test] fn unrelated_test() { assert_eq!(1, 1); }",
        "#[test] fn unrelated_test() { assert_eq!(2, 2); }",
    );
    let got = select(&input);
    subset(&got);
    assert!(got.filterset.contains("tree_reader"));
    assert!(got.filterset.contains("both_roots"));
    assert!(!got.owners.contains("product_only"));
}

#[test]
fn source_root_spelled_relative_or_with_manifest_concat_selects_readers() {
    let mut input = fixture();
    for (name, reader) in [
        (
            "relative_walk",
            "#[test] fn visits() { let _ = std::fs::read_dir(\"src\").unwrap().count(); }",
        ),
        (
            "concat_walk",
            "#[test] fn visits() {
                 let root = std::path::Path::new(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/src\"));
                 let _ = std::fs::read_dir(root).unwrap().count();
             }",
        ),
    ] {
        add_it(&mut input, name, reader);
    }
    let got = select(&input);
    subset(&got);
    for owner in ["relative_walk", "concat_walk"] {
        assert!(got.filterset.contains(owner), "missing walker {owner}");
    }
}

#[test]
fn a_split_literal_path_or_full_path_wrapper_selects_only_the_reader() {
    let mut input = fixture();
    for (name, reader) in [
        (
            "joined_reader",
            "#[test] fn reads() {
                 let root = std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\"));
                 let _ = std::fs::read(root.join(\"src\").join(\"changed.rs\")).unwrap();
             }",
        ),
        (
            "literal_wrapper",
            "fn read(path: &str) -> String {
                 let root = std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\"));
                 std::fs::read_to_string(root.join(path)).unwrap()
             }
             #[test] fn reads() { let _ = read(\"src/changed.rs\"); }",
        ),
        (
            "fixture_walk",
            "#[test] fn visits() {
                 let scratch = super::cli::OwnedScratch::root(\"case\", \"tree\");
                 let _ = std::fs::read_dir(scratch.path()).unwrap().count();
             }",
        ),
    ] {
        add_it(&mut input, name, reader);
    }
    let got = select(&input);
    subset(&got);
    assert!(got.filterset.contains("joined_reader"));
    assert!(got.filterset.contains("literal_wrapper"));
    assert!(!got.owners.contains("fixture_walk"));

    only_edit(
        &mut input,
        "src/indirect.rs",
        "pub fn one() {}",
        "pub fn two() {}",
    );
    let got = select(&input);
    subset(&got);
    assert!(!got.owners.contains("joined_reader"));
    assert!(!got.owners.contains("literal_wrapper"));
}

#[test]
fn a_source_prefix_wrapper_with_a_partial_path_keeps_its_reader() {
    let mut input = fixture();
    put(&mut input, "src/changed.rs", "mod child; pub fn value() {}");
    only_edit(
        &mut input,
        "src/changed/child.rs",
        "pub fn child() -> u8 { 0 }",
        "pub fn child() -> u8 { 1 }",
    );
    add_it(
        &mut input,
        "prefix_wrapper",
        "fn read(name: &str) -> String {
             let root = std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\"));
             std::fs::read_to_string(root.join(\"src\").join(name)).unwrap()
         }
         #[test] fn reads() { let _ = read(\"changed/child.rs\"); }",
    );
    let got = select(&input);
    subset(&got);
    assert!(got.filterset.contains("prefix_wrapper"));
}

#[test]
fn an_unknown_source_view_family_runs_the_whole_lane() {
    let mut input = fixture();
    add_it(
        &mut input,
        "unknown_family",
        "use crate::support::source_view::module_family as family;
         #[test] fn reads_family() {
             let root = std::env::var(\"SOURCE_VIEW_ROOT\").unwrap();
             assert!(!family(&root, false).is_empty());
         }",
    );
    let got = select(&input);
    whole(&got, "source-view");
    assert!(
        got.reasons
            .iter()
            .any(|reason| reason.detail.contains("unknown_family.rs:"))
    );
}

#[test]
fn a_literal_source_family_selects_its_descendants() {
    let mut input = fixture();
    put(
        &mut input,
        "src/changed.rs",
        "mod child; pub fn value() {} ",
    );
    only_edit(
        &mut input,
        "src/changed/child.rs",
        "pub fn child() -> u8 { 0 }",
        "pub fn child() -> u8 { 1 }",
    );
    add_it(
        &mut input,
        "family_reader",
        "#[test] fn reads_child() {
             assert!(super::support::source_view::module_family(\"src/changed.rs\", false)
                 .contains(\"fn child(\"));
         }",
    );
    let got = select(&input);
    subset(&got);
    assert!(got.filterset.contains("family_reader"));

    let root = input.new.get_mut("src/lib.rs").expect("lib root");
    root.push_str(" mod changed_x;");
    only_edit(
        &mut input,
        "src/changed_x.rs",
        "pub fn one() {}",
        "pub fn two() {}",
    );
    let got = select(&input);
    subset(&got);
    assert!(!got.owners.contains("family_reader"));
}

#[test]
fn a_computed_source_read_runs_the_whole_lane() {
    for body in [
        r#"std::fs::read_to_string(format!("src/{name}.rs")).unwrap()"#,
        r#"std::fs::read_to_string(format!("{}/src/{name}.rs", env!("CARGO_MANIFEST_DIR"))).unwrap()"#,
        r#"let root = env!("CARGO_MANIFEST_DIR");
           let path = format!("{root}/src/{name}.rs");
           std::fs::read_to_string(&path).unwrap()"#,
        r#"let mut path = format!("src/{name}");
           path.push_str(".rs");
           std::fs::read_to_string(&path).unwrap()"#,
        r#"let path: String = format!("src/{name}.rs");
           std::fs::read_to_string(&path).unwrap()"#,
    ] {
        let mut input = fixture();
        add_it(
            &mut input,
            "computed_reader",
            &format!(
                r#"#[test] fn reads_path() {{
                let name = std::env::var("SOURCE_FILE").unwrap();
                {body};
            }}"#
            ),
        );
        let got = select(&input);
        assert_eq!(got.verdict, Verdict::Whole, "unplaced read: {body}");
        whole(&got, "source-view");
        assert!(
            got.reasons
                .iter()
                .any(|reason| reason.detail.contains("computed_reader.rs:")),
            "{body}"
        );
    }
}

#[test]
fn a_source_reader_without_a_selectable_test_owner_runs_the_whole_lane() {
    let mut input = fixture();
    add_it(
        &mut input,
        "support",
        "pub(super) mod text_reader;
         mod source_view { pub(super) fn product() -> &'static str { \"product\" } }",
    );
    put(
        &mut input,
        "tests/it/support/text_reader.rs",
        "pub(crate) fn read_text() { let _ = super::source_view::product(); }",
    );
    add_it(
        &mut input,
        "source_consumer",
        "#[test] fn reads_text() { super::support::text_reader::read_text(); }",
    );
    let got = select(&input);
    whole(&got, "source-view");
    assert!(
        got.reasons
            .iter()
            .any(|reason| reason.detail.contains("text_reader.rs:")
                && reason.detail.ends_with("has no selectable test owner"))
    );
}

#[test]
fn an_unresolved_source_include_runs_the_whole_lane() {
    let mut input = fixture();
    add_it(
        &mut input,
        "concat_reader",
        r#"#[test] fn reads_text() {
            let _ = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/", "changed.rs"));
        }"#,
    );
    let got = select(&input);
    whole(&got, "source-view");
    assert!(
        got.reasons
            .iter()
            .any(|reason| reason.detail.ends_with("an include computes a source path"))
    );
}

#[test]
fn parent_module_reexports_keep_direct_consumers() {
    let mut input = fixture();
    put(
        &mut input,
        "src/lib.rs",
        "pub mod changed; pub mod indirect; pub mod parent;",
    );
    put(
        &mut input,
        "src/parent.rs",
        "pub mod changed; pub use changed::value as public_value;",
    );
    add_it(
        &mut input,
        "nested",
        "#[test] fn direct() { let _ = ae::parent::changed::value(); }",
    );
    add_it(
        &mut input,
        "parent_export",
        "#[test] fn exported() { let _ = ae::parent::public_value(); }",
    );
    only_edit(
        &mut input,
        "src/parent/changed.rs",
        "pub fn value() -> u8 { 0 }",
        "pub fn value() -> u8 { 1 }",
    );
    let got = select(&input);
    subset(&got);
    assert!(got.owners.contains("nested"));
    assert!(got.owners.contains("parent_export"));
    assert!(
        !got.owners.contains("direct"),
        "a different module with the same leaf name is not a dependency"
    );
}

#[test]
fn relative_data_manifest_concat_literals_and_basename_joins_select_readers() {
    let mut input = fixture();
    add_it(
        &mut input,
        "relative",
        "#[test] fn reads() { let _ = include_bytes!(\"../../fixtures/message.txt\"); }",
    );
    add_it(
        &mut input,
        "manifest",
        "#[test] fn reads() { let _ = include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/fixtures/message.txt\")); }",
    );
    add_it(
        &mut input,
        "literal",
        "#[test] fn reads() { let _ = \"fixtures/message.txt\"; }",
    );
    add_it(
        &mut input,
        "basename",
        "#[test] fn reads() { let _ = root().join(\"message.txt\"); }",
    );
    input.changes = vec![Change {
        status: Status::Modified,
        path: "fixtures/message.txt".to_owned(),
    }];
    input.old.clear();
    input.tracked.insert("fixtures/message.txt".to_owned());
    let got = select(&input);
    subset(&got);
    for owner in ["relative", "manifest", "literal", "basename"] {
        assert!(got.owners.contains(owner), "missing reader {owner}");
    }
    assert!(!got.doctests);
}

#[test]
fn included_data_makes_its_source_owner_changed() {
    let mut input = fixture();
    put(
        &mut input,
        "src/changed.rs",
        "pub const DATA: &str = include_str!(\"../fixtures/value.txt\"); pub fn value() -> u8 { 1 }",
    );
    input.changes = vec![Change {
        status: Status::Modified,
        path: "fixtures/value.txt".to_owned(),
    }];
    input.old.clear();
    input.tracked.insert("fixtures/value.txt".to_owned());
    let got = select(&input);
    subset(&got);
    assert!(got.owners.contains("direct"));
    assert!(!got.owners.contains("indirect"));
}

#[test]
fn unresolved_paths_are_visible_without_hiding_the_accepted_gap() {
    let mut input = fixture();
    add_it(
        &mut input,
        "macro_reader",
        "#[test] fn reads() { let _ = include_str!(chosen_by_macro!()); }",
    );
    let got = select(&input);
    subset(&got);
    assert!(
        got.why
            .iter()
            .any(|w| w.rule == "unresolved" && w.file == "tests/it/macro_reader.rs")
    );
}

#[test]
fn edited_file_selects_every_real_path_owner() {
    let mut input = fixture();
    input
        .new
        .get_mut("tests/it/main.rs")
        .expect("root")
        .push_str(" #[path = \"piece.rs\"] mod alpha; #[path = \"piece.rs\"] mod beta;");
    let before = "#[test] fn member() { assert_eq!(1, 1); }";
    only_edit(
        &mut input,
        "tests/it/piece.rs",
        before,
        "#[test] fn member() { assert_eq!(2, 2); }",
    );
    let got = select(&input);
    subset(&got);
    assert!(got.owners.contains("alpha"));
    assert!(got.owners.contains("beta"));
    assert!(
        !got.filterset.contains("piece"),
        "the file stem is not an owner"
    );
}

#[test]
fn an_include_fragment_selects_its_containing_namespace() {
    let mut input = fixture();
    add_it(&mut input, "suite", "include!(\"fragment.rs\");");
    only_edit(
        &mut input,
        "tests/it/fragment.rs",
        "#[test] fn included() {}",
        "#[test] fn included() { assert!(true); }",
    );
    let got = select(&input);
    subset(&got);
    assert!(got.owners.contains("suite"));
    assert!(!got.filterset.contains("fragment"));
}

#[test]
fn an_owner_without_tests_is_explained_but_never_becomes_a_dead_prefix() {
    let mut input = fixture();
    add_it(&mut input, "empty", "const VALUE: u8 = 1;");
    only_edit(
        &mut input,
        "tests/it/empty.rs",
        "const VALUE: u8 = 0;",
        "const VALUE: u8 = 1;",
    );
    let got = select(&input);
    subset(&got);
    assert!(!got.filterset.contains("empty"));
    assert!(got.why.iter().any(|w| w.owner == "empty"));
    assert_eq!(got.total, 7);
}

#[test]
fn a_private_import_change_in_a_shared_rig_is_not_a_test_only_edit() {
    let mut input = shared_fixture();
    add_it(&mut input, "first", "pub fn value() -> u8 { 1 }");
    add_it(&mut input, "second", "pub fn value() -> u8 { 2 }");
    let before = "use crate::first::value; pub fn helper() -> u8 { value() } #[test] fn local() { let _ = helper(); }";
    let test_only = "use crate::first::value; pub fn helper() -> u8 { value() } #[test] fn local() { assert_eq!(helper(), 1); }";
    only_edit(&mut input, "tests/it/rig.rs", before, test_only);
    subset(&select(&input));
    let import_only = "use crate::second::value; pub fn helper() -> u8 { value() } #[test] fn local() { let _ = helper(); }";
    only_edit(&mut input, "tests/it/rig.rs", before, import_only);
    whole(&select(&input), "shared-rig");
}

#[test]
fn an_added_module_in_a_shared_rig_is_a_non_test_item_change() {
    let mut input = shared_fixture();
    let before = input.new["tests/it/rig.rs"].clone();
    only_edit(
        &mut input,
        "tests/it/rig.rs",
        &before,
        &format!("{before} mod added;"),
    );
    put(
        &mut input,
        "tests/it/rig/added.rs",
        "#[test] fn new_case() {}",
    );
    input.changes.push(Change {
        status: Status::Added,
        path: "tests/it/rig/added.rs".to_owned(),
    });
    whole(&select(&input), "shared-rig");
}

fn shared_fixture() -> Input {
    let mut input = fixture();
    add_it(
        &mut input,
        "rig",
        "pub fn helper() -> u8 { 1 } #[test] fn local() { assert_eq!(helper(), 1); }",
    );
    add_it(
        &mut input,
        "consumer",
        "#[test] fn consumer() { let _ = crate::rig::helper(); }",
    );
    input
}

#[test]
fn shared_rig_test_only_edit_is_local_but_helper_edit_is_whole() {
    let before = "pub fn helper() -> u8 { 1 } #[test] fn local() { assert_eq!(helper(), 1); }";
    let mut input = shared_fixture();
    only_edit(
        &mut input,
        "tests/it/rig.rs",
        before,
        "pub fn helper() -> u8 { 1 } #[test] fn local() { assert!(helper() > 0); }",
    );
    let got = select(&input);
    subset(&got);
    assert!(got.owners.contains("rig"));
    assert!(!got.owners.contains("consumer"));
    only_edit(
        &mut input,
        "tests/it/rig.rs",
        before,
        "pub fn helper() -> u8 { 2 } #[test] fn local() { assert_eq!(helper(), 1); }",
    );
    whole(&select(&input), "shared-rig");
}

#[test]
fn removing_a_test_attribute_cannot_claim_the_test_only_exception() {
    let mut input = shared_fixture();
    only_edit(
        &mut input,
        "tests/it/rig.rs",
        "pub fn helper() -> u8 { 1 } #[test] fn local() {}",
        "pub fn helper() -> u8 { 1 } fn local() {}",
    );
    whole(&select(&input), "shared-rig");
}

#[test]
fn an_unshared_helper_edit_stays_local() {
    let mut input = fixture();
    let before = "fn helper() -> u8 { 0 } #[test] fn local() { let _ = helper(); }";
    add_it(&mut input, "local", before);
    only_edit(
        &mut input,
        "tests/it/local.rs",
        before,
        "fn helper() -> u8 { 1 } #[test] fn local() { let _ = helper(); }",
    );
    let got = select(&input);
    subset(&got);
    assert!(got.owners.contains("local"));
}

#[test]
fn only_a_new_module_declaration_targeting_an_added_file_is_a_cheap_root_edit() {
    let mut input = fixture();
    let before = input.new["src/lib.rs"].clone();
    let after = format!("{before} pub mod fresh;");
    only_edit(&mut input, "src/lib.rs", &before, &after);
    put(&mut input, "src/fresh.rs", "pub fn fresh() {}");
    input.changes.push(Change {
        status: Status::Added,
        path: "src/fresh.rs".to_owned(),
    });
    add_it(
        &mut input,
        "fresh_user",
        "#[test] fn fresh_test() { ae::fresh::fresh(); }",
    );
    let got = select(&input);
    subset(&got);
    assert!(got.owners.contains("fresh_user"));
    for root_after in [
        "pub mod changed;",
        "#[cfg(unix)] pub mod changed; pub mod indirect;",
        "pub mod changed; pub mod indirect; pub const EXTRA: u8 = 0;",
    ] {
        let mut input = fixture();
        only_edit(&mut input, "src/lib.rs", &before, root_after);
        whole(&select(&input), "crate-root");
    }
}

#[test]
fn retargeting_a_module_path_or_changing_an_owner_set_is_whole() {
    let mut input = fixture();
    add_it(&mut input, "holder", "#[path = \"one.rs\"] mod child;");
    put(&mut input, "tests/it/one.rs", "#[test] fn first() {}");
    put(&mut input, "tests/it/two.rs", "#[test] fn second() {}");
    only_edit(
        &mut input,
        "tests/it/holder.rs",
        "#[path = \"one.rs\"] mod child;",
        "#[path = \"two.rs\"] mod child;",
    );
    whole(&select(&input), "module-map");
}

#[test]
fn every_build_configuration_and_selector_trigger_names_its_reason() {
    for (path, reason) in [
        ("Cargo.toml", "build-input"),
        ("Cargo.lock", "build-input"),
        ("build.rs", "build-input"),
        ("rust-toolchain.toml", "build-input"),
        (".cargo/config.toml", "build-input"),
        ("justfile", "justfile"),
        (".config/nextest.toml", "config"),
        ("src/main.rs", "crate-root"),
        ("examples/affected.rs", "selector-source"),
        ("tests/it/gate/affected.rs", "selector-source"),
        ("tests/it/gate/modtree.rs", "selector-source"),
        ("assets/unreferenced.dat", "unknown-path"),
    ] {
        let mut input = fixture();
        only_edit(&mut input, path, "", "");
        let got = select(&input);
        whole(&got, reason);
        assert!(
            got.reasons
                .iter()
                .any(|r| r.id == reason && r.detail.contains(path)),
            "reason must name {path}"
        );
    }
}

#[test]
fn deletions_renames_copies_and_macro_item_changes_are_whole() {
    for status in [
        Status::Deleted,
        Status::Renamed {
            from: "src/previous.rs".to_owned(),
        },
        Status::Copied {
            from: "src/previous.rs".to_owned(),
        },
    ] {
        let mut input = fixture();
        let deleted = matches!(status, Status::Deleted);
        input.changes[0].status = status;
        whole(&select(&input), if deleted { "deleted" } else { "renamed" });
    }
    let mut input = fixture();
    only_edit(
        &mut input,
        "src/changed.rs",
        "macro_rules! answer { () => { 0 } }",
        "macro_rules! answer { () => { 1 } }",
    );
    whole(&select(&input), "macro");
}

#[test]
fn unreferenced_documentation_fuzz_and_the_mac_manifest_do_not_force_whole() {
    for path in [
        "docs/guide.md",
        "README.md",
        "fuzz/corpus/sample",
        ".config/macos-subset",
    ] {
        let mut input = fixture();
        input.changes = vec![Change {
            status: Status::Modified,
            path: path.to_owned(),
        }];
        input.old.clear();
        input.tracked.insert(path.to_owned());
        let got = select(&input);
        subset(&got);
        assert_eq!(
            got.owners,
            BTreeSet::from(["doors".to_owned(), "floor".to_owned(), "gate".to_owned()])
        );
        assert_eq!((got.not_selected, got.total), (4, 7));
        assert!(!got.doctests);
    }
    let mut input = fixture();
    input.changes = vec![Change {
        status: Status::Modified,
        path: "install".to_owned(),
    }];
    input.old.clear();
    assert!(
        select(&input).check,
        "install changes carry the shell check flag even on a whole verdict"
    );
}

#[test]
fn a_module_prefix_never_selects_a_sibling_with_a_similar_name() {
    let mut input = fixture();
    add_it(&mut input, "direct_extra", "#[test] fn sibling() {}");
    let got = select(&input);
    subset(&got);
    assert!(!got.owners.contains("direct_extra"));
    assert!(
        got.filterset.contains(")::/"),
        "owner alternatives require a :: boundary"
    );
    assert_eq!((got.not_selected, got.total), (4, 8));
    assert_eq!(render(&got), render(&select(&input)));
}

#[test]
fn change_order_cannot_change_the_filter_or_explanation_order() {
    let mut input = fixture();
    input.old.insert(
        "src/indirect.rs".to_owned(),
        "pub fn value() -> u8 { 0 }".to_owned(),
    );
    input.changes.push(Change {
        status: Status::Modified,
        path: "src/indirect.rs".to_owned(),
    });
    let first = select(&input);
    subset(&first);
    assert!(first.owners.contains("direct"));
    assert!(first.owners.contains("indirect"));
    input.changes.reverse();
    assert_eq!(render(&first), render(&select(&input)));
}

#[test]
fn zero_statically_enumerated_tests_produce_a_zero_counter() {
    let mut input = fixture();
    for (path, text) in &mut input.new {
        if path.starts_with("tests/it/") && path != "tests/it/main.rs" {
            text.clear();
        }
    }
    let got = select(&input);
    assert!(matches!(got.verdict, Verdict::Subset));
    assert_eq!((got.not_selected, got.total), (0, 0));
}

fn record(tag: &str, payload: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(format!("{tag} {}\n", payload.len()).as_bytes());
    out.extend_from_slice(payload);
    out.push(b'\n');
}

fn codec_records() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("version", b"1".to_vec()),
        ("mode", b"gate".to_vec()),
        ("base", b"main".to_vec()),
        ("merge-base", "a".repeat(40).into_bytes()),
        ("head", "b".repeat(40).into_bytes()),
        ("change", b"M\0src/lib.rs".to_vec()),
        ("tracked", b"src/lib.rs".to_vec()),
        (
            "new",
            "src/lib.rs\0// café\npub fn value() {}\n"
                .as_bytes()
                .to_vec(),
        ),
        ("old", b"src/lib.rs\0".to_vec()),
    ]
}

fn encode(records: &[(&str, Vec<u8>)], completed: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (tag, payload) in records {
        record(tag, payload, &mut bytes);
    }
    if completed {
        record("end", records.len().to_string().as_bytes(), &mut bytes);
    }
    bytes
}

fn stream_error(bytes: &[u8]) {
    match decode(bytes) {
        Ok(_) => panic!("malformed stream was accepted"),
        Err(reason) => assert_eq!(reason.id, "stream"),
    }
}

#[test]
fn the_stream_uses_byte_lengths_and_keeps_empty_old_content() {
    let decoded = decode(&encode(&codec_records(), true))
        .unwrap_or_else(|r| panic!("valid stream: {} {}", r.id, r.detail));
    assert!(matches!(decoded.mode, Mode::Gate));
    assert_eq!(decoded.base, "main");
    assert_eq!(decoded.merge_base, "a".repeat(40));
    assert_eq!(decoded.head, "b".repeat(40));
    assert_eq!(decoded.new["src/lib.rs"], "// café\npub fn value() {}\n");
    assert_eq!(decoded.old["src/lib.rs"], "");
    assert_eq!(decoded.changes.len(), 1);
    assert!(matches!(decoded.changes[0].status, Status::Modified));
    assert_eq!(decoded.changes[0].path, "src/lib.rs");
}

#[test]
fn non_source_changes_need_no_content_but_modified_sources_need_old_and_new() {
    for path in [
        "docs/a.md",
        "Cargo.toml",
        "justfile",
        "build.rs",
        "fuzz/fuzz_targets/p.rs",
        "examples/affected.rs",
    ] {
        let mut records = codec_records();
        records.truncate(5);
        records.push(("change", format!("M\0{path}").into_bytes()));
        records.push(("tracked", path.as_bytes().to_vec()));
        assert!(
            decode(&encode(&records, true)).is_ok(),
            "non-source {path} needs no content"
        );
    }
    for omitted in ["old", "new"] {
        let mut records = codec_records();
        records.retain(|(tag, _)| *tag != omitted);
        stream_error(&encode(&records, true));
    }
}

#[test]
fn malformed_framing_order_hashes_and_completion_fail_closed() {
    let valid = encode(&codec_records(), true);
    for cut in 0..valid.len() {
        stream_error(&valid[..cut]);
    }
    let mut trailing = valid.clone();
    trailing.push(b'x');
    stream_error(&trailing);
    let mut bad = codec_records();
    bad[0].0 = "unknown";
    stream_error(&encode(&bad, true));
    let mut bad = codec_records();
    bad.swap(1, 2);
    stream_error(&encode(&bad, true));
    for hash in ["a".repeat(39), "g".repeat(40), "a".repeat(41)] {
        let mut bad = codec_records();
        bad[3].1 = hash.into_bytes();
        stream_error(&encode(&bad, true));
    }
    let mut count = encode(&codec_records(), false);
    record("end", b"8", &mut count);
    stream_error(&count);
    stream_error(&encode(&codec_records(), false));
    for header in [
        b"version 01\n1\n".as_slice(),
        b"version -1\n1\n",
        b"version x\n1\n",
        b"version 2\n1\n",
        b"version 999999999999999999999999999999\n",
    ] {
        let mut bad = header.to_vec();
        bad.extend_from_slice(&valid[b"version 1\n1\n".len()..]);
        stream_error(&bad);
    }
}

#[test]
fn duplicate_paths_invalid_utf8_and_spurious_old_records_fail_closed() {
    for tag in ["change", "tracked", "new", "old"] {
        let mut bad = codec_records();
        let index = bad
            .iter()
            .position(|(t, _)| *t == tag)
            .expect("record class");
        bad.insert(index, (tag, bad[index].1.clone()));
        stream_error(&encode(&bad, true));
    }
    for tag in ["tracked", "new", "old"] {
        let mut bad = codec_records();
        let index = bad
            .iter()
            .position(|(t, _)| *t == tag)
            .expect("record class");
        bad[index].1.push(0xff);
        stream_error(&encode(&bad, true));
    }
    let mut bad = codec_records();
    bad.last_mut().expect("old").1 = b"src/other.rs\0old".to_vec();
    stream_error(&encode(&bad, true));
    let mut bad = codec_records();
    bad[5].1 = b"R100\0src/previous.rs\0src/lib.rs".to_vec();
    stream_error(&encode(&bad, true));
}

#[test]
fn untracked_changes_are_preview_only_and_rename_status_keeps_its_old_path() {
    let mut records = codec_records();
    records[1].1 = b"preview".to_vec();
    records[5].1 = b"U\0src/lib.rs".to_vec();
    records.retain(|(tag, _)| *tag != "old");
    let decoded =
        decode(&encode(&records, true)).unwrap_or_else(|r| panic!("preview: {}", r.detail));
    assert!(matches!(decoded.mode, Mode::Preview));
    assert!(matches!(decoded.changes[0].status, Status::Untracked));
    records[1].1 = b"gate".to_vec();
    stream_error(&encode(&records, true));
    records[5].1 = b"R100\0src/previous.rs\0src/lib.rs".to_vec();
    let decoded =
        decode(&encode(&records, true)).unwrap_or_else(|r| panic!("rename: {}", r.detail));
    assert!(
        matches!(&decoded.changes[0].status, Status::Renamed { from } if from == "src/previous.rs")
    );
}

#[test]
fn the_subset_protocol_is_exact_and_carries_the_static_gap_counter() {
    let got = render(&Selection {
        verdict: Verdict::Subset,
        reasons: vec![],
        owners: BTreeSet::from(["gate".to_owned(), "doors".to_owned(), "floor".to_owned()]),
        why: vec![Why {
            owner: "gate".to_owned(),
            file: "tests/it/gate.rs".to_owned(),
            line: 9,
            rule: "own",
            target: "tests/it/gate.rs".to_owned(),
        }],
        doctests: true,
        check: false,
        not_selected: 4,
        total: 7,
        filterset: "kind(lib) | kind(bin) | (binary_id(ae::it) & test(/^(doors|floor|gate)::/))"
            .to_owned(),
    });
    assert_eq!(
        got,
        "version 1\nverdict subset\nowner doors\nowner floor\nowner gate\nwhy gate tests/it/gate.rs:9 own tests/it/gate.rs\ndoctests yes\ncheck no\ncounter 4 7\nfilterset kind(lib) | kind(bin) | (binary_id(ae::it) & test(/^(doors|floor|gate)::/))\nend\n"
    );
}

#[test]
fn the_whole_protocol_has_a_reason_and_no_subset_claims_even_at_zero_inventory() {
    let got = render(&Selection {
        verdict: Verdict::Whole,
        reasons: vec![Reason {
            id: "stream",
            detail: "missing end".to_owned(),
        }],
        owners: BTreeSet::new(),
        why: vec![],
        doctests: false,
        check: true,
        not_selected: 0,
        total: 0,
        filterset: String::new(),
    });
    assert_eq!(
        got,
        "version 1\nverdict whole\nreason stream missing end\ndoctests no\ncheck yes\nend\n"
    );
}
