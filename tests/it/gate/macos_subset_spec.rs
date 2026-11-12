//! Independent P1 acceptance: lead ruling F3-C and plan r2.
//! These controls derive membership from synthetic Rust source, not a saved inventory.
use super::macos_subset::{self as subset, Class, Entry, Finding};
use super::modtree::{self, Crate, Files, ModPath};

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

fn entries(text: &str) -> Vec<Entry> {
    subset::parse_manifest(text).expect("contract-valid manifest")
}

fn owners(files: &Files) -> std::collections::BTreeMap<String, Vec<ModPath>> {
    modtree::module_map(files).expect("all declared modules resolve")
}

fn uncovered(findings: &[Finding], class: Class, module: &str) -> bool {
    findings.iter().any(|finding| {
        matches!(finding,
        Finding::UncoveredSite(site) if site.class == class && site.module.path == module)
    })
}

#[test]
fn a_valid_control_and_source_derived_test_inventory_are_not_vacuous() {
    let files = tree(&[]);
    assert_eq!(
        subset::test_names(&files, &owners(&files)),
        ["plain::control"]
    );
    assert!(subset::check(&entries("unit\nmodule plain\n"), &files).is_empty());
    assert!(subset::sites(&files, &owners(&files)).is_empty());
}

#[test]
fn every_marker_class_detects_a_new_site_and_names_its_owner() {
    for (class, body) in [
        (
            Class::OsConditional,
            r#"let _ = cfg!(target_os = "macos");"#,
        ),
        (Class::ProcessTable, r#"let _ = ["ps", "-Ao", "pid"]; "#),
        (Class::TmuxClient, r#"let _ = ["tmux", "attach-session"]; "#),
        (Class::Recipe, r#"let _ = ["just", "rust-test"]; "#),
        (Class::Fsync, r"file.sync_all();"),
        (Class::Clipboard, r#"let _ = ["pbcopy"]; "#),
        (Class::ArchiveTool, r#"let _ = ["tar", "-czf", "bundle"]; "#),
    ] {
        let source = format!("#[test]\nfn planted() {{ {body} }}\n");
        let files = tree(&[
            ("tests/it/main.rs", "mod plain; mod fresh;"),
            ("tests/it/fresh.rs", &source),
        ]);
        let found = subset::check(&entries("unit\nmodule plain\n"), &files);
        assert!(uncovered(&found, class, "fresh"), "{class:?}: {found:?}");
        assert!(
            subset::check(&entries("unit\nmodule plain\nmodule fresh\n"), &files).is_empty(),
            "a covered planted site must pass: {class:?}"
        );
        assert!(
            subset::sites(&files, &owners(&files))
                .iter()
                .any(|site| site.class == class
                    && site.file == "tests/it/fresh.rs"
                    && site.line == 2)
        );
    }
}

#[test]
fn comments_and_unrelated_identifiers_do_not_create_boundary_sites() {
    let files = tree(&[(
        "tests/it/plain.rs",
        r"
// target_os, sync_all, attach-session, pbcopy, sysctl, codesign
#[test] fn control() { let sync_all_count = 1; let _ = sync_all_count; }
",
    )]);
    assert!(subset::sites(&files, &owners(&files)).is_empty());
}

#[test]
fn unit_sites_need_no_integration_prefix_but_unit_is_mandatory_once() {
    let files = tree(&[(
        "src/lib.rs",
        "#[cfg(target_os = \"macos\")] fn os() {} #[test] fn unit_test() {}",
    )]);
    assert!(subset::check(&entries("unit\n"), &files).is_empty());
    for bad in ["", "# empty\n", "module plain\n", "unit\nunit\n"] {
        assert!(subset::parse_manifest(bad).is_err(), "must refuse {bad:?}");
    }
}

#[test]
fn module_mapping_preserves_every_owner_and_checks_each_it_owner() {
    let files = tree(&[
        (
            "src/lib.rs",
            "#[path = \"../tests/it/shared.rs\"] mod unit_copy;",
        ),
        (
            "tests/it/main.rs",
            "#[path = \"shared.rs\"] mod alpha; #[path = \"shared.rs\"] mod beta;",
        ),
        (
            "tests/it/shared.rs",
            "#[test] fn boundary() { let _ = cfg!(target_os = \"macos\"); }",
        ),
    ]);
    let map = owners(&files);
    assert_eq!(
        map["tests/it/shared.rs"],
        [
            ModPath {
                krate: Crate::Lib,
                path: "unit_copy".into()
            },
            ModPath {
                krate: Crate::It,
                path: "alpha".into()
            },
            ModPath {
                krate: Crate::It,
                path: "beta".into()
            },
        ]
    );
    assert_eq!(
        subset::test_names(&files, &map),
        ["alpha::boundary", "beta::boundary"]
    );
    let found = subset::check(&entries("unit\nmodule alpha\n"), &files);
    assert!(uncovered(&found, Class::OsConditional, "beta"), "{found:?}");
    assert!(
        !uncovered(&found, Class::OsConditional, "alpha"),
        "{found:?}"
    );
    assert!(subset::check(&entries("unit\nmodule alpha\nmodule beta\n"), &files).is_empty());
}

#[test]
fn inline_path_and_include_children_keep_their_actual_module_paths() {
    let files = tree(&[
        ("tests/it/main.rs", "mod outer;"),
        (
            "tests/it/outer.rs",
            r#"
mod inline {
    #[path = "../special.rs"] mod relocated;
}
mod included { include!("body.rs"); }
"#,
        ),
        (
            "tests/it/outer/special.rs",
            "#[test] fn os() { let _ = cfg!(target_os = \"macos\"); }",
        ),
        (
            "tests/it/body.rs",
            "#[test] fn archive() { let _ = [\"codesign\"]; }",
        ),
    ]);
    let found = subset::check(&entries("unit\n"), &files);
    assert!(
        uncovered(&found, Class::OsConditional, "outer::inline::relocated"),
        "{found:?}"
    );
    assert!(
        uncovered(&found, Class::ArchiveTool, "outer::included"),
        "{found:?}"
    );
    assert!(
        subset::check(
            &entries("unit\nmodule outer::inline::relocated\nmodule outer::included\n"),
            &files
        )
        .is_empty()
    );
}

#[test]
fn unresolved_modules_and_stale_module_or_exact_test_members_refuse() {
    let files = tree(&[]);
    for text in [
        "unit\nmodule absent\n",
        "unit\ntest plain::absent\n",
        "unit\nhistory absent # 330\n",
    ] {
        let found = subset::check(&entries(text), &files);
        assert!(
            found.iter().any(|f| matches!(f, Finding::UnknownModule(_))),
            "{found:?}"
        );
    }
    let broken = tree(&[("tests/it/main.rs", "mod absent;")]);
    let found = subset::check(&entries("unit\n"), &broken);
    assert!(
        found
            .iter()
            .any(|f| matches!(f, Finding::UnresolvedTree(_))),
        "{found:?}"
    );
}

fn helper_tree(test_body: &str) -> Files {
    let caller = format!(
        "use super::rig::Rig;\n#[test] fn drives() {{ {test_body} }}\n#[test] fn unrelated() {{}}\n"
    );
    tree(&[
        (
            "tests/it/main.rs",
            "mod plain; mod raw; mod rig; mod client;",
        ),
        ("tests/it/raw.rs", "pub fn run() { Command::new(\"ae\"); }"),
        (
            "tests/it/rig.rs",
            r"
pub struct Rig;
impl Rig {
    pub fn launch() { Self::inner(); }
    fn inner() { local(); }
}
fn local() { super::raw::run(); }
",
        ),
        ("tests/it/client.rs", &caller),
    ])
}

#[test]
fn a_new_shared_helper_requires_its_own_driving_exact_member() {
    let files = helper_tree("Rig::launch();");
    let map = owners(&files);
    let helpers = subset::helpers(&files, &map);
    assert!(
        helpers.iter().any(|h| h.file == "tests/it/rig.rs"
            && h.item == "Rig::launch"
            && h.callers.contains(&"client::drives".into())),
        "{helpers:?}"
    );
    let found = subset::check(&entries("unit\nmodule raw\n"), &files);
    assert!(
        found
            .iter()
            .any(|f| matches!(f, Finding::HelperWithoutMember(h)
        if h.file == "tests/it/rig.rs" && h.item == "Rig::launch")),
        "{found:?}"
    );
    let wrong = subset::check(
        &entries("unit\nmodule raw\nhelper tests/it/rig.rs::Rig::launch = client::unrelated\n"),
        &files,
    );
    assert!(
        wrong
            .iter()
            .any(|f| matches!(f, Finding::MemberNotCaller { item, member }
        if item == "tests/it/rig.rs::Rig::launch" && member == "client::unrelated")),
        "{wrong:?}"
    );
    let stale = subset::check(
        &entries("unit\nmodule raw\nhelper tests/it/gone.rs::run = client::drives\n"),
        &files,
    );
    assert!(
        stale.iter().any(|f| matches!(f, Finding::StaleHelper(_))),
        "{stale:?}"
    );
}

#[test]
fn same_file_self_calls_and_import_aliases_resolve_without_bare_name_collisions() {
    let files = tree(&[
        (
            "tests/it/main.rs",
            "mod raw; mod rig; mod client; mod harmless;",
        ),
        ("tests/it/raw.rs", "pub fn run() { Command::new(\"ae\"); }"),
        (
            "tests/it/rig.rs",
            r"
use super::raw::run as execute;
pub struct Rig;
impl Rig {
    pub fn launch(&self) { self.inner(); }
    fn inner(&self) { execute(); }
}
",
        ),
        ("tests/it/harmless.rs", "pub fn run() {}"),
        (
            "tests/it/client.rs",
            r"
#[test] fn drives() { super::rig::Rig.launch(); }
#[test] fn name_collision() { super::harmless::run(); }
",
        ),
    ]);
    let helpers = subset::helpers(&files, &owners(&files));
    let launch = helpers
        .iter()
        .find(|h| h.file == "tests/it/rig.rs" && h.item == "Rig::launch")
        .expect("self.inner plus imported execute reaches the primitive");
    assert!(
        launch.callers.contains(&"client::drives".into()),
        "{helpers:?}"
    );
    assert!(
        !launch.callers.contains(&"client::name_collision".into()),
        "{helpers:?}"
    );
}

#[test]
fn actual_construction_reaches_drop_but_merely_naming_the_type_does_not() {
    let files = tree(&[
        ("tests/it/main.rs", "mod raw; mod rig; mod client;"),
        (
            "tests/it/raw.rs",
            "pub fn kill() { Command::new(\"kill\"); }",
        ),
        (
            "tests/it/rig.rs",
            r"
pub struct Rig { pub number: u8 }
impl Rig { pub fn new() -> Self { Self { number: 0 } } }
impl Drop for Rig { fn drop(&mut self) { super::raw::kill(); } }
",
        ),
        (
            "tests/it/client.rs",
            r"
#[test] fn constructs() { let _rig = super::rig::Rig::new(); }
#[test] fn literal() { let _rig = super::rig::Rig { number: 1 }; }
#[test] fn names_only() { let _ = std::mem::size_of::<super::rig::Rig>(); }
",
        ),
    ]);
    let helpers = subset::helpers(&files, &owners(&files));
    let kill = helpers
        .iter()
        .find(|h| h.file == "tests/it/raw.rs" && h.item == "kill")
        .expect("Drop makes kill a shared boundary helper");
    for member in ["client::constructs", "client::literal"] {
        assert!(kill.callers.contains(&member.into()), "{helpers:?}");
    }
    assert!(
        !kill.callers.contains(&"client::names_only".into()),
        "{helpers:?}"
    );
}

#[test]
fn a_child_mode_noop_is_not_a_driving_member_in_a_normal_run() {
    let files = tree(&[
        ("tests/it/main.rs", "mod raw; mod client;"),
        ("tests/it/raw.rs", "pub fn run() { Command::new(\"ae\"); }"),
        (
            "tests/it/client.rs",
            r#"
#[test] fn normal() { super::raw::run(); }
#[test] fn child() {
    let Some(_mode) = std::env::var_os("AE_SPEC_CHILD_ONLY") else { return; };
    super::raw::run();
}
"#,
        ),
    ]);
    let helpers = subset::helpers(&files, &owners(&files));
    let run = helpers
        .iter()
        .find(|h| h.file == "tests/it/raw.rs" && h.item == "run")
        .expect("the normal test drives run");
    assert!(
        run.callers.contains(&"client::normal".into()),
        "{helpers:?}"
    );
    assert!(
        !run.callers.contains(&"client::child".into()),
        "{helpers:?}"
    );
}

#[test]
fn manifest_exact_members_never_widen_into_the_whole_module() {
    let selected = entries(
        "unit\nmodule boundary\n\
         test cheap::one\nhelper tests/it/rig.rs::Rig::launch = client::drives\n\
         history transport # 332\nhistory archive::regression # 330\n",
    );
    let filter = subset::filterset(&selected);
    for text in [
        "kind(lib)",
        "kind(bin)",
        "binary_id(ae::it)",
        "test(=cheap::one)",
        "test(=client::drives)",
        "test(=archive::regression)",
    ] {
        assert!(filter.contains(text), "{text}: {filter}");
    }
    assert!(
        !filter.contains("cheap::/") && !filter.contains("client::/"),
        "{filter}"
    );
    for bad in [
        "unit\nsurprise value\n",
        "unit\nmodule x); all()\n",
        "unit\nhelper missing_separator\n",
        "unit\nhistory transport\n",
    ] {
        assert!(subset::parse_manifest(bad).is_err(), "must refuse {bad:?}");
    }
}

#[test]
fn a_new_tmux_client_group_member_requires_coverage_without_a_source_marker() {
    let files = tree(&[(
        ".config/nextest.toml",
        r"
[test-groups.tmux-clients]
max-threads = 1
[[profile.default.overrides]]
filter = 'test(/^plain::/)'
test-group = 'tmux-clients'
",
    )]);
    let found = subset::check(&entries("unit\n"), &files);
    assert!(uncovered(&found, Class::TmuxClient, "plain"), "{found:?}");
    assert!(subset::check(&entries("unit\nmodule plain\n"), &files).is_empty());
}

#[test]
fn an_unparsed_tmux_client_filter_fails_closed_and_seat_groups_stay_outside_this_class() {
    let clients = tree(&[(
        ".config/nextest.toml",
        r"
[[profile.default.overrides]]
filter = 'test(/plain/)'
test-group = 'tmux-clients'
",
    )]);
    let found = subset::check(&entries("unit\nmodule plain\n"), &clients);
    assert!(
        found
            .iter()
            .any(|f| matches!(f, Finding::GroupFilter { .. })),
        "{found:?}"
    );
    let seats = tree(&[(
        ".config/nextest.toml",
        r"
[[profile.default.overrides]]
filter = 'test(/plain/)'
test-group = 'tmux-seats'
",
    )]);
    assert!(subset::check(&entries("unit\n"), &seats).is_empty());
}

// The filter text contract uses exact tests and anchored module prefixes.
// This small oracle does not execute cargo from inside the running test binary.
fn text_selects(filter: &str, name: &str) -> bool {
    if filter.contains(&format!("test(={name})")) {
        return true;
    }
    filter.split("test(/^").skip(1).any(|tail| {
        let Some((pattern, _)) = tail.split_once("/)") else {
            return false;
        };
        let Some(prefixes) = pattern.strip_suffix("::") else {
            return false;
        };
        let prefixes = prefixes
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .unwrap_or(prefixes);
        prefixes
            .split('|')
            .any(|prefix| name.starts_with(&format!("{prefix}::")))
    })
}

#[test]
fn history_accepts_resolving_modules_and_renders_both_prefix_and_exact_forms() {
    let files = tree(&[]);
    let selected = entries("unit\nhistory plain # 330\n");
    assert!(subset::check(&selected, &files).is_empty());
    let filter = subset::filterset(&selected);
    assert!(filter.contains("test(=plain)"), "{filter}");
    assert!(text_selects(&filter, "plain::control"), "{filter}");
    let exact = subset::filterset(&entries("unit\nhistory plain::control # 330\n"));
    assert!(exact.contains("test(=plain::control)"), "{exact}");
    assert!(!text_selects("test(=plain::other)", "plain::control"));
    assert!(!text_selects("test(/^other::/)", "plain::control"));
}

#[test]
fn ordinary_trailing_comments_and_history_issue_separators_are_distinct() {
    let parsed = entries(
        "# full line\n\nunit # unit tests\nmodule plain # marker\n\
        test plain::control # exact\n\
        helper tests/it/rig.rs::Rig::launch = plain::control # member\n\
        history plain # 330\n",
    );
    assert_eq!(
        parsed,
        vec![
            Entry::Unit,
            Entry::Module("plain".into()),
            Entry::Test("plain::control".into()),
            Entry::Helper {
                item: "tests/it/rig.rs::Rig::launch".into(),
                member: "plain::control".into()
            },
            Entry::History {
                target: "plain".into(),
                issue: "330".into()
            }
        ]
    );
}

fn real_files() -> Files {
    fn visit(root: &std::path::Path, directory: &std::path::Path, files: &mut Files) {
        for entry in std::fs::read_dir(directory).expect("source directory") {
            let entry = entry.expect("source entry");
            let path = entry.path();
            if entry.file_type().expect("source entry type").is_dir() {
                visit(root, &path, files);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let key = path
                    .strip_prefix(root)
                    .expect("repository-relative source")
                    .to_string_lossy()
                    .replace('\\', "/");
                files.insert(key, std::fs::read_to_string(path).expect("Rust source"));
            }
        }
    }
    let root = super::root();
    let mut files = Files::new();
    visit(&root, &root.join("src"), &mut files);
    visit(&root, &root.join("tests"), &mut files);
    files.insert(
        ".config/nextest.toml".into(),
        super::read(&root.join(".config/nextest.toml")),
    );
    files
}

#[test]
fn the_real_manifest_covers_the_real_tree_and_preserves_named_selection_controls() {
    let files = real_files();
    let manifest = super::read(&super::root().join(".config/macos-subset"));
    let selected = entries(&manifest);
    let found = subset::check(&selected, &files);
    assert!(
        found.is_empty(),
        "the shipped guard must accept the shipped manifest: {found:#?}"
    );
    let names = subset::test_names(&files, &owners(&files));
    let filter = subset::filterset(&selected);
    for name in [
        "menu::a_server_the_cleanup_cannot_kill_keeps_its_scratch",
        "autotrust_spec::trustmiss_watchdog_answers_only_the_recorded_folder_and_names_off_or_foreign",
        "transport::sc_017k_one_real_query_answers_present_absent_and_prefix_candidates_by_exact_name",
        "session_launch::seat_profile_flags_refuse_unknown_words_before_session_state_is_written",
    ] {
        assert!(
            names.iter().any(|known| known == name),
            "stale positive control: {name}"
        );
        assert!(
            text_selects(&filter, name),
            "the real filter must select {name}: {filter}"
        );
    }
    let non_boundary = "cliparse_spec::top_level_help_version_option_and_session_name_pins";
    assert!(
        names.iter().any(|known| known == non_boundary),
        "stale negative control"
    );
    assert!(
        !text_selects(&filter, non_boundary),
        "the pure parser module stays outside the Mac subset: {filter}"
    );
    assert!(
        !filter.contains("all()") && !filter.contains("test(/.*/)"),
        "{filter}"
    );
    assert!(selected.iter().any(|entry| matches!(entry, Entry::Helper { item, member }
        if item == "tests/it/session_launch.rs::Rig::launch" &&
        member == "session_launch::seat_profile_flags_refuse_unknown_words_before_session_state_is_written")));
}
