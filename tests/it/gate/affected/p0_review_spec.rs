//! Driver regression cases from the P0 r1 review (layout-nav, review-20261118T154641Z-286790c2).
//! Every path is assembled from pieces, so this file names no repository data dependency.

use super::super::modtree::Files;
use super::{Change, Input, Mode, Status, Verdict, select};

const SRC: &str = "src";
const IT: &str = "tests/it";

fn source(module: &str) -> String {
    format!("{SRC}/{module}.rs")
}

fn it(name: &str) -> String {
    format!("{IT}/{name}.rs")
}

fn tree(pairs: &[(String, &str)]) -> Files {
    pairs
        .iter()
        .map(|(path, text)| (path.clone(), (*text).to_owned()))
        .collect()
}

#[test]
fn p0_review_a_flat_root_reexport_of_a_submodule_keeps_its_reexport_consumer() {
    // A flat tree: `pub use app::draw;` is an ordinary root re-export, not a layer alias, so an
    // edit of the parent module still selects the consumer that spells `ae::draw`.
    let edited = source("app");
    let new = tree(&[
        (source("lib"), "pub mod app; pub use app::draw;"),
        (source("main"), "fn main() {}"),
        (edited.clone(), "pub mod draw; pub fn run() -> u8 { 1 }"),
        (source("app/draw"), "pub fn draw() {}"),
        (
            it("main"),
            "mod doors; mod floor; mod gate; mod reexported; mod unrelated;",
        ),
        (it("doors"), "#[test] fn door_guard() {}"),
        (it("floor"), "#[test] fn floor_guard() {}"),
        (it("gate"), "#[test] fn gate_guard() {}"),
        (
            it("reexported"),
            "#[test] fn reexported() { ae::draw::draw(); }",
        ),
        (it("unrelated"), "#[test] fn unrelated() {}"),
    ]);
    let input = Input {
        mode: Mode::Gate,
        base: "main".to_owned(),
        merge_base: "a".repeat(40),
        head: "b".repeat(40),
        tracked: new.keys().cloned().collect(),
        old: tree(&[(edited.clone(), "pub mod draw; pub fn run() -> u8 { 0 }")]),
        new,
        changes: vec![Change {
            status: Status::Modified,
            path: edited,
        }],
    };

    let got = select(&input);
    assert!(
        matches!(got.verdict, Verdict::Subset),
        "a function edit in a flat module is a subset"
    );
    assert!(
        got.why
            .iter()
            .any(|pick| pick.owner == "reexported" && pick.rule == "reexport"),
        "the consumer of the root re-export `ae::draw` must stay selected: {:?}",
        got.why
    );
    assert!(!got.owners.contains("unrelated"));
}
