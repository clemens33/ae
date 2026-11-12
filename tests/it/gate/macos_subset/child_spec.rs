//! The existing env-is-none child pattern is a no-op in a normal nextest run.

use super::super::modtree::{self, Files};
use super::{Entry, Finding, check, helpers};

#[test]
fn an_env_is_none_child_is_not_a_driving_member() {
    let files = Files::from([
        ("src/lib.rs".into(), String::new()),
        ("src/main.rs".into(), "fn main() {}".into()),
        ("tests/it/main.rs".into(), "mod raw; mod client;".into()),
        (
            "tests/it/raw.rs".into(),
            r#"pub fn run() { let _ = std::process::Command::new("ae"); }"#.into(),
        ),
        (
            "tests/it/client.rs".into(),
            r#"
#[test] fn child() {
    if std::env::var_os("AE_GATE_STALE_LANE_OWNER_CHILD").is_none() {
        return;
    }
    super::raw::run();
}
#[test] fn drives() { super::raw::run(); }
"#
            .into(),
        ),
    ]);
    let owners = modtree::module_map(&files).expect("all fixture modules resolve");
    let found = helpers(&files, &owners);
    let helper = found
        .iter()
        .find(|helper| helper.file == "tests/it/raw.rs" && helper.item == "run")
        .expect("the normal driving test keeps the boundary helper visible");
    assert!(
        helper
            .callers
            .iter()
            .any(|caller| caller == "client::drives")
    );
    let entries = |member: &str| {
        vec![
            Entry::Unit,
            Entry::Helper {
                item: "tests/it/raw.rs::run".into(),
                member: member.into(),
            },
        ]
    };
    assert!(check(&entries("client::drives"), &files).is_empty());
    let findings = check(&entries("client::child"), &files);
    println!(
        "child callers: {:?}; findings: {findings:?}",
        helper.callers
    );
    assert!(
        findings.iter().any(
            |finding| matches!(finding, Finding::MemberNotCaller { item, member }
            if item == "tests/it/raw.rs::run" && member == "client::child")
        ),
        "an env-gated child that returns in a normal run cannot represent this helper"
    );
}
