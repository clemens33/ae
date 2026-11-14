//! Contract split guards. Registered below `doors`, inheriting its existing
//! reasoned source/config read allowance; no new process or filesystem door.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::support::source_view;

const LIMIT: usize = 22 * 1024;
const REFERENCE: &str = "docs/reference/contract.md";
const AUTOMATION: &str = "Automation features are allowed again, but the focus stays on code quality, architecture and guardrails: a new one must earn its place by daily use.";
const OLD_PAUSE: &str = "New automation features are paused until a daily-use inventory exists. Only the human may lift this pause.";
const MODULE_MAP: &str = "Module map: [Structure](docs/reference/contract.md#structure), layer map: [Layer map](docs/reference/contract.md#layer-map).";
const TOPICS: [&str; 7] = [
    "Structure",
    "Session helpers",
    "Agent tool capabilities",
    "Environment doors",
    "GNU vs BSD — check the command YOU are about to type",
    "Key invariants",
    "Config",
];

fn read(path: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
        .unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn size_ok(agents: &str) -> bool {
    !agents.trim().is_empty() && agents.len() <= LIMIT
}

fn automation_ok(agents: &str) -> bool {
    agents.lines().any(|line| line == AUTOMATION)
}

// Deliberately a bounded grammar: explicit src/*.rs file owners, optionally
// followed by ::Rust::identifiers. Every occurrence is checked, including prose
// and code spans; an empty owner inventory is a refusal, never a vacuous pass.
fn owners(text: &str) -> Vec<(usize, String, String)> {
    let mut found = Vec::new();
    for (line, text) in text.lines().enumerate() {
        for (start, _) in text.match_indices("src/") {
            let tail = &text[start..];
            let Some(end) = tail.find(".rs") else {
                continue;
            };
            let path = &tail[..end + 3];
            if !path.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'-' | b'.')
            }) {
                continue;
            }
            let mut suffix = &tail[end + 3..];
            if let Some(group) = suffix.strip_prefix("::{") {
                let end = group
                    .find('}')
                    .unwrap_or_else(|| panic!("unclosed owner group: {tail}"));
                for member in group[..end].split(',') {
                    let member = member.trim();
                    assert!(
                        !member.is_empty()
                            && member
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric()
                                    || matches!(byte, b'_' | b':')),
                        "bad owner group: {tail}"
                    );
                    found.push((line + 1, path.to_owned(), member.to_owned()));
                }
                continue;
            }
            let mut segments = Vec::new();
            while let Some(rest) = suffix.strip_prefix("::") {
                let length = rest
                    .bytes()
                    .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                    .count();
                assert!(length > 0, "malformed owner at line {}: {tail}", line + 1);
                segments.push(&rest[..length]);
                suffix = &rest[length..];
            }
            found.push((line + 1, path.to_owned(), segments.join("::")));
        }
    }
    found
}

fn insert(set: &mut BTreeSet<String>, prefix: &str, name: &str) -> String {
    let name = if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}::{name}")
    };
    set.insert(name.clone());
    name
}

fn declared(items: &[syn::Item], prefix: &str, set: &mut BTreeSet<String>) {
    for item in items {
        match item {
            syn::Item::Fn(item) => {
                insert(set, prefix, &item.sig.ident.to_string());
            }
            syn::Item::Const(item) => {
                insert(set, prefix, &item.ident.to_string());
            }
            syn::Item::Static(item) => {
                insert(set, prefix, &item.ident.to_string());
            }
            syn::Item::Type(item) => {
                insert(set, prefix, &item.ident.to_string());
            }
            syn::Item::Struct(item) => {
                let own = insert(set, prefix, &item.ident.to_string());
                for field in &item.fields {
                    if let Some(name) = &field.ident {
                        insert(set, &own, &name.to_string());
                    }
                }
            }
            syn::Item::Enum(item) => {
                let own = insert(set, prefix, &item.ident.to_string());
                for variant in &item.variants {
                    insert(set, &own, &variant.ident.to_string());
                }
            }
            syn::Item::Union(item) => {
                insert(set, prefix, &item.ident.to_string());
            }
            syn::Item::Trait(item) => {
                let own = insert(set, prefix, &item.ident.to_string());
                for member in &item.items {
                    if let syn::TraitItem::Fn(member) = member {
                        insert(set, &own, &member.sig.ident.to_string());
                    }
                }
            }
            syn::Item::Mod(item) => {
                let own = insert(set, prefix, &item.ident.to_string());
                if let Some((_, items)) = &item.content {
                    declared(items, &own, set);
                }
            }
            syn::Item::Impl(item) => {
                if let syn::Type::Path(path) = item.self_ty.as_ref()
                    && let Some(name) = path.path.segments.last()
                {
                    let own = if prefix.is_empty() {
                        name.ident.to_string()
                    } else {
                        format!("{prefix}::{}", name.ident)
                    };
                    for member in &item.items {
                        match member {
                            syn::ImplItem::Fn(member) => {
                                insert(set, &own, &member.sig.ident.to_string());
                            }
                            syn::ImplItem::Const(member) => {
                                insert(set, &own, &member.ident.to_string());
                            }
                            syn::ImplItem::Type(member) => {
                                insert(set, &own, &member.ident.to_string());
                            }
                            _ => {}
                        }
                    }
                }
            }
            syn::Item::Macro(item) => {
                if let Some(name) = &item.ident {
                    insert(set, prefix, &name.to_string());
                }
            }
            _ => {}
        }
    }
}

fn source_symbols(path: &str, source: &str) -> BTreeSet<String> {
    // Classify first: unsupported test gates or unparseable Rust fail closed.
    // Named tests in the map are legitimate owners too, so parse the shared
    // view's comment-free WHOLE source after classification, not just production.
    source_view::classify(path, source).unwrap_or_else(|failure| panic!("{failure}"));
    let text =
        source_view::without_comments(path, source).unwrap_or_else(|failure| panic!("{failure}"));
    let parsed = syn::parse_file(&text).unwrap_or_else(|error| panic!("{path}: {error}"));
    let mut set = BTreeSet::new();
    declared(&parsed.items, "", &mut set);
    set
}

fn owner_errors(text: &str, sources: &BTreeMap<String, BTreeSet<String>>) -> Vec<String> {
    let owners = owners(text);
    if owners.is_empty() {
        return vec!["no explicit src/*.rs owners".to_owned()];
    }
    owners
        .into_iter()
        .filter_map(|(line, path, name)| match sources.get(&path) {
            None => Some(format!("line {line}: missing owner file {path}")),
            Some(symbols) if !name.is_empty() && !symbols.contains(&name) => {
                Some(format!("line {line}: missing owner {path}::{name}"))
            }
            Some(_) => None,
        })
        .collect()
}

fn slug(heading: &str) -> String {
    heading
        .chars()
        .flat_map(char::to_lowercase)
        .filter_map(|character| {
            if character.is_alphanumeric() || matches!(character, '-' | '_') {
                Some(character)
            } else if character == ' ' {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

fn headings(text: &str) -> BTreeSet<String> {
    let mut fence = None;
    let mut counts = BTreeMap::<String, usize>::new();
    let mut result = BTreeSet::new();
    for line in text.lines() {
        let line = line.trim_start();
        if line.starts_with("```") || line.starts_with("~~~") {
            let mark = &line[..3];
            if fence == Some(mark) {
                fence = None;
            } else if fence.is_none() {
                fence = Some(mark);
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        let level = line.bytes().take_while(|byte| *byte == b'#').count();
        if !(1..=6).contains(&level) || !line[level..].starts_with(' ') {
            continue;
        }
        let title = line[level..].trim().trim_end_matches('#').trim();
        let base = slug(title);
        let count = counts.entry(base.clone()).or_default();
        result.insert(if *count == 0 {
            base
        } else {
            format!("{base}-{count}")
        });
        *count += 1;
    }
    result
}

fn index_errors(agents: &str, reference: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let targets = headings(reference);
    let mut links = BTreeSet::new();
    for (start, _) in agents.match_indices("](docs/reference/contract.md") {
        let rest = &agents[start + 2..];
        let Some(end) = rest.find(')') else {
            errors.push("unclosed reference link".to_owned());
            continue;
        };
        let link = &rest[..end];
        let Some(anchor) = link.strip_prefix("docs/reference/contract.md#") else {
            errors.push(format!("reference index link needs a heading: {link}"));
            continue;
        };
        links.insert(anchor.to_owned());
        if !targets.contains(anchor) {
            errors.push(format!("missing reference heading #{anchor}"));
        }
    }
    for topic in TOPICS {
        if !links.contains(&slug(topic)) {
            errors.push(format!("topic absent from index: {topic}"));
        }
    }
    if !agents
        .lines()
        .take(22)
        .any(|line| line == "Before touching a module, read its matching topic below.")
    {
        errors.push("reading rule missing from first 22 lines".to_owned());
    }
    if !agents.lines().take(22).any(|line| line == MODULE_MAP) {
        errors.push("module map pointer missing from first 22 lines".to_owned());
    }
    errors
}

#[test]
fn the_contract_fits_the_startup_budget() {
    let agents = read("AGENTS.md");
    assert!(
        size_ok(&agents),
        "AGENTS.md: {} bytes exceeds {LIMIT}, or empty",
        agents.len()
    );
    assert_eq!(
        read("CLAUDE.md"),
        "@AGENTS.md\n",
        "CLAUDE.md still imports the contract"
    );
}

#[test]
fn the_contract_keeps_the_automation_ruling() {
    assert!(
        automation_ok(&read("AGENTS.md")),
        "AGENTS.md must retain the exact automation ruling: {AUTOMATION}"
    );
}

#[test]
fn every_contract_index_entry_names_a_real_reference_heading() {
    let errors = index_errors(&read("AGENTS.md"), &read(REFERENCE));
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn every_explicit_contract_owner_exists() {
    let sources = source_view::tree()
        .iter()
        .filter(|(path, _)| path.starts_with("src/"))
        .map(|(path, text)| (path.clone(), source_symbols(path, text)))
        .collect();
    for path in ["AGENTS.md", REFERENCE] {
        let errors = owner_errors(&read(path), &sources);
        assert!(errors.is_empty(), "{path}:\n{}", errors.join("\n"));
    }
}

#[test]
fn the_split_guards_reject_independent_planted_edits() {
    assert!(automation_ok(AUTOMATION));
    assert!(!automation_ok(""));
    assert!(!automation_ok(OLD_PAUSE));
    assert!(!automation_ok(&AUTOMATION.replace(
        "Automation features are allowed again",
        "New automation features are paused"
    )));
    assert!(!automation_ok(&AUTOMATION.replace(
        "code quality, architecture and guardrails",
        "shipping speed"
    )));
    assert!(!automation_ok(&AUTOMATION.replace(
        "must earn its place by daily use",
        "needs no justification"
    )));
    assert!(size_ok("# ae\n"));
    assert!(!size_ok(&"x".repeat(LIMIT + 1)));
    assert!(!size_ok(""));
    // The module-map line also links the layer map, a subsection of Structure.
    let mut reference = "### Layer map\n".to_owned();
    let mut agents = "Before touching a module, read its matching topic below.\n".to_owned();
    agents.push_str(MODULE_MAP);
    agents.push('\n');
    for topic in TOPICS {
        reference.push_str("## ");
        reference.push_str(topic);
        reference.push('\n');
        agents.push_str("- [");
        agents.push_str(topic);
        agents.push_str("](docs/reference/contract.md#");
        agents.push_str(&slug(topic));
        agents.push_str(")\n");
    }
    assert!(index_errors(&agents, &reference).is_empty());
    assert!(!index_errors(&agents.replace("#config)", "#missing)"), &reference).is_empty());
    assert!(!index_errors(&agents, &reference.replace("## Config", "## Gone")).is_empty());
    assert!(!index_errors(&agents, &reference.replace("### Layer map", "### Gone")).is_empty());
    assert!(
        !index_errors(
            &agents.replace("Before touching", "After touching"),
            &reference
        )
        .is_empty()
    );
    assert!(!index_errors(&agents.replace("Module map:", "Modules:"), &reference).is_empty());
    assert!(!index_errors("", &reference).is_empty());
    assert!(!headings("```text\n## Hidden\n```\n").contains("hidden"));
    let sources = BTreeMap::from([(
        "src/a.rs".to_owned(),
        source_symbols(
            "src/a.rs",
            r#"
        // fn phantom() {}
        const EXAMPLE: &str = "fn invented() {}";
        fn real() {}
        struct A;
        impl A { fn live() {} }
        #[cfg(test)] mod tests { fn named_test() {} }
    "#,
        ),
    )]);
    assert!(
        owner_errors(
            "src/a.rs::real src/a.rs::A::live src/a.rs::tests::named_test",
            &sources
        )
        .is_empty()
    );
    for changed in [
        "src/missing.rs",
        "src/a.rs::gone",
        "src/a.rs::A::gone",
        "src/a.rs::phantom",
        "src/a.rs::invented",
        "",
    ] {
        assert!(
            !owner_errors(changed, &sources).is_empty(),
            "missed mutation: {changed}"
        );
    }
}
