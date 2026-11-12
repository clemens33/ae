//! Independent supplemental P1 controls for actual driving members and TOML group keys.

use super::super::modtree::{self, Files};
use super::{Entry, Finding, NEXTEST, check, helpers};

fn tree(extra: &[(&str, &str)]) -> Files {
    let mut files = Files::from([
        ("src/lib.rs".into(), String::new()),
        ("src/main.rs".into(), "fn main() {}".into()),
        ("tests/it/main.rs".into(), "mod plain;".into()),
        ("tests/it/plain.rs".into(), "#[test] fn control() {}".into()),
    ]);
    for (path, text) in extra {
        files.insert((*path).into(), (*text).into());
    }
    files
}

#[test]
fn a_same_named_method_on_another_receiver_is_not_a_driving_member() {
    let files = tree(&[
        ("tests/it/main.rs", "mod raw; mod rig; mod client;"),
        (
            "tests/it/raw.rs",
            r#"pub fn run() { let _ = std::process::Command::new("ae"); }"#,
        ),
        (
            "tests/it/rig.rs",
            "pub struct Rig; impl Rig { pub fn launch() { super::raw::run(); } }",
        ),
        (
            "tests/it/client.rs",
            r"
use super::rig::Rig;
struct Fake;
impl Fake { fn launch(&self) {} }
#[test] fn drives() { Rig::launch(); }
#[test] fn unrelated() { Fake.launch(); }
",
        ),
    ]);
    let owners = modtree::module_map(&files).expect("all fixture modules resolve");
    let found = helpers(&files, &owners);
    let rig = found
        .iter()
        .find(|h| h.file == "tests/it/rig.rs" && h.item == "Rig::launch")
        .expect("the shared boundary helper must be discovered");
    println!("Rig::launch callers: {:?}", rig.callers);
    assert!(rig.callers.iter().any(|name| name == "client::drives"));
    let raw = found
        .iter()
        .find(|h| h.file == "tests/it/raw.rs" && h.item == "run")
        .expect("the boundary primitive wrapper must be discovered");
    assert!(raw.callers.iter().any(|name| name == "client::drives"));
    let mut entries = vec![Entry::Unit];
    for helper in &found {
        entries.push(Entry::Helper {
            item: format!("{}::{}", helper.file, helper.item),
            member: if helper.file == "tests/it/rig.rs" && helper.item == "Rig::launch" {
                "client::unrelated".into()
            } else {
                "client::drives".into()
            },
        });
    }
    let findings = check(&entries, &files);
    println!("check with unrelated Rig member: {findings:?}");
    assert!(
        findings
            .iter()
            .any(|f| matches!(f, Finding::MemberNotCaller { item, member }
            if item == "tests/it/rig.rs::Rig::launch" && member == "client::unrelated")),
        "the guard certified a test that only calls Fake::launch as a Rig::launch driving member"
    );
}

#[test]
fn a_quoted_toml_group_key_must_not_hide_a_client_boundary() {
    let config = "[test-groups.tmux-clients]\nmax-threads = 1\n[[profile.default.overrides]]\nfilter = 'test(/^plain::/)'\ntest-group = 'tmux-clients'\n";
    let mut files = tree(&[(NEXTEST, config)]);
    let entries = vec![Entry::Unit];
    let canonical = check(&entries, &files);
    println!("bare key findings: {canonical:?}");
    assert!(
        canonical
            .iter()
            .any(|f| matches!(f, Finding::UncoveredSite(site)
            if site.module.path == "plain")),
        "the positive control must detect the group member"
    );
    files.insert(
        NEXTEST.into(),
        config.replace("test-group =", "'test-group' ="),
    );
    let quoted = check(&entries, &files);
    println!("quoted key findings: {quoted:?}");
    assert!(
        !quoted.is_empty(),
        "valid TOML quoted keys must preserve membership or fail closed"
    );
}
