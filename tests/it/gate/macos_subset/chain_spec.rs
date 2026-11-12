//! A driving member must reach the helper through the actual receiver type.

use super::super::modtree::{self, Files};
use super::{Entry, Finding, check, helpers};

fn findings_for(rig: &str, client: &str) -> Vec<Finding> {
    let files = Files::from([
        ("src/lib.rs".into(), String::new()),
        ("src/main.rs".into(), "fn main() {}".into()),
        (
            "tests/it/main.rs".into(),
            "mod raw; mod rig; mod client;".into(),
        ),
        (
            "tests/it/raw.rs".into(),
            r#"pub fn run() { let _ = std::process::Command::new("ae"); }"#.into(),
        ),
        ("tests/it/rig.rs".into(), rig.into()),
        ("tests/it/client.rs".into(), client.into()),
    ]);
    let owners = modtree::module_map(&files).expect("the fixture tree resolves");
    let found = helpers(&files, &owners);
    let rig = found
        .iter()
        .find(|helper| helper.file == "tests/it/rig.rs" && helper.item == "Rig::launch")
        .expect("the shared boundary helper must remain visible");
    assert!(rig.callers.iter().any(|caller| caller == "client::drives"));
    println!("Rig::launch callers: {:?}", rig.callers);
    let mut entries = vec![Entry::Unit];
    for helper in found {
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
    findings
}

fn refuses_member(findings: &[Finding]) -> bool {
    findings.iter().any(|finding| {
        matches!(finding, Finding::MemberNotCaller { item, member }
            if item == "tests/it/rig.rs::Rig::launch" && member == "client::unrelated")
    })
}

#[test]
fn an_inherent_clone_returning_another_type_does_not_drive_the_original_type() {
    let findings = findings_for(
        r"
pub struct Rig;
pub struct Fake;
impl Rig {
    pub fn new() -> Self { Rig }
    pub fn clone(&self) -> Fake { Fake }
    pub fn launch(&self) { super::raw::run(); }
}
impl Fake { pub fn launch(&self) {} }
",
        r"
use super::rig::Rig;
#[test] fn drives() { Rig::new().launch(); }
#[test] fn unrelated() { Rig::new().clone().launch(); }
",
    );
    assert!(
        refuses_member(&findings),
        "the declared clone return type is Fake, so this test never calls Rig::launch"
    );
}

#[test]
fn an_option_receiver_does_not_drive_its_inner_type_without_unwrapping() {
    let findings = findings_for(
        r"
pub struct Rig;
impl Rig { pub fn launch(&self) { super::raw::run(); } }
pub fn maybe() -> Option<Rig> { None }
pub trait Inert { fn launch(&self); }
impl Inert for Option<Rig> { fn launch(&self) {} }
",
        r"
use super::rig::{Rig, Inert, maybe};
#[test] fn drives() { Rig.launch(); }
#[test] fn unrelated() { maybe().launch(); }
",
    );
    assert!(
        refuses_member(&findings),
        "the Option method handles None and never constructs or calls the inner Rig"
    );
}
