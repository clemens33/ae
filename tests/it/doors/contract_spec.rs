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
    rust_symbols(path, source)
}

// The declared items of one comment-free Rust file, without the product
// classification (a `tests/` file is not product code).
fn rust_symbols(path: &str, source: &str) -> BTreeSet<String> {
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

// ---- the Key invariants owner pin ---------------------------------------------
//
// The table is parsed from the doc at test time, so the pin follows its edits:
// there is no hand-copied list. Each code span of an Owner cell, read left to
// right, is one of these references, and each must resolve:
//   `src/…/f.rs` or `tests/…/f.rs`  a file of the tree; it joins the row's named files
//   `src/…/f.rs::a::b`              that file declares `a::b`
//   `::a::b`                        a file the row named before it declares `a::b`
//   `f.rs`, `f.rs::a::b`            some `src/…/f.rs` exists (and declares `a::b`)
//   `x::a`                          a file the row named before it declares `x::a`
//                                   (a type member such as `Kind::Fleet`), or a module
//                                   file `src/…/x.rs` declares `a`; deeper paths split
//                                   at each `::` (`x::y::a`: `src/…/x/y.rs` declares `a`)
//   `crate::x::a`                   `src/lib.rs` declares `x::a`, or a module file as above
// A span without `::` that is none of the file forms is prose (a word, a command,
// a non-Rust path) and unchecked. Every row names at least one existing file. A
// malformed table, row, span or path fails; a parse never passes over nothing.

const INVARIANTS: &str = "## Key invariants";
const INVARIANTS_HEADER: &str = "| Invariant | Owner |";

type Symbols = BTreeMap<String, BTreeSet<String>>;

// One table row: its 1-based line in the doc, its invariant and its owner cell.
struct Row {
    line: usize,
    invariant: String,
    owner: String,
}

#[derive(Clone, Copy)]
enum Reference<'a> {
    File(&'a str, Option<&'a str>),
    Continuation(&'a str),
    Basename(&'a str, Option<&'a str>),
    // The path, and whether it was spelled from `crate::`.
    Path(&'a str, bool),
}

#[derive(Clone, Copy)]
enum Table {
    Before,
    Header,
    Rows,
    After,
}

fn is_ident(word: &str) -> bool {
    let mut bytes = word.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn is_item_path(path: &str) -> bool {
    path.split("::").all(is_ident)
}

// The cells of a table line, split at every pipe that no backslash escapes, as
// GitHub does even inside a code span. `None` for a line outside any table.
fn table_cells(line: &str) -> Option<Vec<&str>> {
    let inner = line.trim_end().strip_prefix('|')?;
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    let bytes = inner.as_bytes();
    let mut cells = Vec::new();
    let mut start = 0;
    for (at, byte) in bytes.iter().enumerate() {
        if *byte == b'|' && (at == 0 || bytes[at - 1] != b'\\') {
            cells.push(inner[start..at].trim());
            start = at + 1;
        }
    }
    cells.push(inner[start..].trim());
    Some(cells)
}

fn is_separator(cells: &[&str]) -> bool {
    cells.len() == 2
        && cells
            .iter()
            .all(|cell| !cell.is_empty() && cell.bytes().all(|byte| matches!(byte, b'-' | b':')))
}

// The rows of the one Key invariants table, and every structural failure.
fn invariant_rows(doc: &str) -> (Vec<Row>, Vec<String>) {
    let lines: Vec<&str> = doc.lines().collect();
    let starts: Vec<usize> = (0..lines.len())
        .filter(|&index| lines[index] == INVARIANTS)
        .collect();
    let &[start] = starts.as_slice() else {
        let found = starts.len();
        let why = format!("{REFERENCE}: expected one `{INVARIANTS}` heading, found {found}");
        return (Vec::new(), vec![why]);
    };
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    let mut table = Table::Before;
    for (index, line) in lines.iter().enumerate().skip(start + 1) {
        if line.starts_with("# ") || line.starts_with("## ") {
            break;
        }
        let number = index + 1;
        match (table, table_cells(line)) {
            (Table::Before | Table::After, None) => {}
            (Table::Before, Some(_)) => {
                if *line != INVARIANTS_HEADER {
                    errors.push(format!(
                        "{REFERENCE}:{number}: the table header is not `{INVARIANTS_HEADER}`"
                    ));
                }
                table = Table::Header;
            }
            (Table::Header, cells) => {
                if !cells.is_some_and(|cells| is_separator(&cells)) {
                    errors.push(format!(
                        "{REFERENCE}:{number}: the header is not followed by a separator row"
                    ));
                }
                table = Table::Rows;
            }
            (Table::Rows, None) => table = Table::After,
            (Table::Rows, Some(cells)) => {
                if let [invariant, owner] = cells[..] {
                    rows.push(Row {
                        line: number,
                        invariant: invariant.to_owned(),
                        owner: owner.to_owned(),
                    });
                } else {
                    let found = cells.len();
                    errors.push(format!(
                        "{REFERENCE}:{number}: a row has {found} cells, not 2: escape a literal pipe as \\|"
                    ));
                }
            }
            (Table::After, Some(_)) => errors.push(format!(
                "{REFERENCE}:{number}: a table row after the table ended"
            )),
        }
    }
    if rows.is_empty() {
        errors.push(format!("{REFERENCE}: the Key invariants table has no rows"));
    }
    (rows, errors)
}

// The single-backtick code spans of one cell.
fn code_spans(cell: &str) -> Result<Vec<&str>, String> {
    let parts: Vec<&str> = cell.split('`').collect();
    if parts.len() % 2 == 0 {
        return Err("an unclosed code span".to_owned());
    }
    let spans: Vec<&str> = parts.into_iter().skip(1).step_by(2).collect();
    if spans.contains(&"") {
        return Err("an empty or double-backtick code span".to_owned());
    }
    Ok(spans)
}

fn owner_reference(span: &str) -> Result<Option<Reference<'_>>, String> {
    let (head, item) = span
        .split_once("::")
        .map_or((span, None), |(head, item)| (head, Some(item)));
    if item.is_some_and(|item| !is_item_path(item)) {
        return Err("a malformed item path".to_owned());
    }
    if head.starts_with("src/") || head.starts_with("tests/") {
        let rust = Path::new(head).extension().is_some_and(|ext| ext == "rs");
        if !rust && item.is_none() {
            return Ok(None);
        }
        let plain = head
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'-' | b'.'));
        if !rust || !plain {
            return Err("a malformed file path".to_owned());
        }
        return Ok(Some(Reference::File(head, item)));
    }
    if let Some(stem) = head.strip_suffix(".rs") {
        if is_ident(stem) {
            return Ok(Some(Reference::Basename(stem, item)));
        }
        return Err("a file is spelled src/…, tests/… or a bare f.rs".to_owned());
    }
    let Some(item) = item else {
        return Ok(None);
    };
    if head.is_empty() {
        Ok(Some(Reference::Continuation(item)))
    } else if head == "crate" {
        Ok(Some(Reference::Path(item, true)))
    } else if is_ident(head) {
        Ok(Some(Reference::Path(span, false)))
    } else {
        Err("not an owner path".to_owned())
    }
}

fn declares(known: &Symbols, path: &str, item: &str) -> bool {
    known.get(path).is_some_and(|items| items.contains(item))
}

fn src_files_ending<'a>(known: &'a Symbols, suffix: &'a str) -> impl Iterator<Item = &'a String> {
    known
        .keys()
        .filter(move |path| path.starts_with("src/") && path.ends_with(suffix))
}

fn any_declares(known: &Symbols, paths: &[String], item: &str) -> bool {
    paths.iter().any(|path| declares(known, path, item))
}

fn resolve(found: Reference<'_>, named: &mut Vec<String>, known: &Symbols) -> Result<(), String> {
    match found {
        Reference::File(path, item) => {
            if !known.contains_key(path) {
                return Err("no such file".to_owned());
            }
            if !named.iter().any(|seen| seen == path) {
                named.push(path.to_owned());
            }
            match item {
                Some(item) if !declares(known, path, item) => {
                    Err(format!("{path} declares no {item}"))
                }
                _ => Ok(()),
            }
        }
        Reference::Continuation(_) if named.is_empty() => {
            Err("a continuation before the row names a file".to_owned())
        }
        Reference::Continuation(item) => {
            if any_declares(known, named, item) {
                Ok(())
            } else {
                Err(format!(
                    "no file the row named before it declares {item} (named: {})",
                    named.join(", ")
                ))
            }
        }
        Reference::Basename(stem, item) => {
            let suffix = format!("/{stem}.rs");
            let candidates: Vec<&String> = src_files_ending(known, &suffix).collect();
            if candidates.is_empty() {
                return Err(format!("no src/…/{stem}.rs"));
            }
            match item {
                Some(item) if !candidates.iter().any(|path| declares(known, path, item)) => {
                    Err(format!("no src/…/{stem}.rs declares {item}"))
                }
                _ => Ok(()),
            }
        }
        Reference::Path(path, rooted) => {
            if (rooted && declares(known, "src/lib.rs", path))
                || (!rooted && any_declares(known, named, path))
            {
                return Ok(());
            }
            let segments: Vec<&str> = path.split("::").collect();
            for split in 1..segments.len() {
                let suffix = format!("/{}.rs", segments[..split].join("/"));
                let rest = segments[split..].join("::");
                if src_files_ending(known, &suffix).any(|file| declares(known, file, &rest)) {
                    return Ok(());
                }
            }
            Err(format!(
                "neither a file the row named before it nor a module file declares {path}"
            ))
        }
    }
}

// `(rows, references)` checked, or every failure, each with its doc line and
// the start of its row.
fn invariant_errors(doc: &str, known: &Symbols) -> Result<(usize, usize), Vec<String>> {
    let (rows, mut errors) = invariant_rows(doc);
    let mut checked = 0;
    for row in &rows {
        let excerpt: String = row.invariant.chars().take(60).collect();
        let line = row.line;
        let mut fail = |what: &str| {
            errors.push(format!("{REFERENCE}:{line}: {what} [row: {excerpt}…]"));
        };
        let spans = match code_spans(&row.owner) {
            Ok(spans) => spans,
            Err(why) => {
                fail(&why);
                continue;
            }
        };
        let mut named = Vec::new();
        for span in spans {
            let outcome = owner_reference(span).and_then(|found| match found {
                Some(found) => {
                    checked += 1;
                    resolve(found, &mut named, known)
                }
                None => Ok(()),
            });
            if let Err(why) = outcome {
                fail(&format!("`{span}`: {why}"));
            }
        }
        if named.is_empty() {
            fail("the owner cell names no existing src/ or tests/ file");
        }
    }
    if errors.is_empty() {
        Ok((rows.len(), checked))
    } else {
        Err(errors)
    }
}

// Every `src/` file, and every `tests/` file the doc names, with its items.
fn tree_symbols(doc: &str) -> Symbols {
    source_view::tree()
        .iter()
        .filter_map(|(path, text)| {
            if path.starts_with("src/") {
                Some((path.clone(), source_symbols(path, text)))
            } else if doc.contains(path.as_str()) {
                Some((path.clone(), rust_symbols(path, text)))
            } else {
                None
            }
        })
        .collect()
}

// The first owner reference of each form in the table, as `(line, span)`.
fn first_of_each_form(doc: &str) -> BTreeMap<&'static str, (usize, String)> {
    let mut first = BTreeMap::new();
    for row in invariant_rows(doc).0 {
        for span in code_spans(&row.owner).unwrap_or_default() {
            let form = match owner_reference(span) {
                Ok(Some(Reference::File(_, Some(_)))) => "file item",
                Ok(Some(Reference::Continuation(_))) => "continuation",
                Ok(Some(Reference::Basename(_, Some(_)))) => "basename item",
                Ok(Some(Reference::Path(..))) => "path",
                _ => continue,
            };
            first.entry(form).or_insert((row.line, span.to_owned()));
        }
    }
    first
}

// `doc` with the last occurrence of the code span `span` on line `line`
// renamed to `<span>_gone`.
fn renamed_on(doc: &str, line: usize, span: &str) -> String {
    let quoted = format!("`{span}`");
    let mut out = Vec::new();
    for (index, text) in doc.lines().enumerate() {
        if index + 1 == line {
            let (before, after) = text
                .rsplit_once(&quoted)
                .unwrap_or_else(|| panic!("line {line} has no {quoted}"));
            out.push(format!("{before}`{span}_gone`{after}"));
        } else {
            out.push(text.to_owned());
        }
    }
    out.join("\n")
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

#[test]
fn every_key_invariant_owner_resolves() {
    let doc = read(REFERENCE);
    let known = tree_symbols(&doc);
    let (rows, checked) = invariant_errors(&doc, &known).unwrap_or_else(|errors| {
        panic!(
            "{} stale Key invariants owner(s):\n{}",
            errors.len(),
            errors.join("\n")
        )
    });
    // An independent, simpler count: every table line of the section but the
    // header and the separator is a parsed row.
    let table_lines = doc
        .lines()
        .skip_while(|line| *line != INVARIANTS)
        .skip(1)
        .take_while(|line| !line.starts_with("## ") && !line.starts_with("# "))
        .filter(|line| line.starts_with('|'))
        .count();
    assert_eq!(rows + 2, table_lines, "rows parsed vs table lines");
    assert!(
        checked >= rows,
        "{rows} rows but {checked} owner references"
    );
    eprintln!("Key invariants: {rows} rows, {checked} owner references resolved");
    // A scratch copy of the doc with one owner of each form renamed fails on
    // exactly that owner.
    let first = first_of_each_form(&doc);
    for form in ["file item", "continuation"] {
        assert!(first.contains_key(form), "no {form} owner in the table");
    }
    for (form, (line, span)) in first {
        let errors = invariant_errors(&renamed_on(&doc, line, &span), &known)
            .err()
            .unwrap_or_default();
        let needle = format!("{REFERENCE}:{line}: `{span}_gone`");
        assert!(
            errors.len() == 1 && errors[0].starts_with(&needle),
            "{form} {span}: {errors:?}"
        );
    }
}

const ROW_ONE: &str = "| One `a \\| b` rule | `src/l2_model/watchdog.rs::done_progress` the episode, \
    `::wait_progress` its rung, `Banner::Codex` the row; `tracked::AddressGuard::check`, \
    `run.rs::resumable`, `end.rs` and `crate::run`; `ae doctor` and \
    `src/l3_services/render/{core,worker}.txt` are prose |";
const ROW_TWO: &str = "| Two rule | `src/l2_model/tracked.rs` (`AddressGuard::check`), pinned by `tests/it/floor.rs::pins_it` |";

// Each planted edit of the fixture, and the text its failure must contain.
const PLANTED: [(&str, &str, &str); 27] = [
    (
        "`::wait_progress`",
        "`::wait_gone`",
        ":8: `::wait_gone`: no file the row named",
    ),
    (
        "::done_progress`",
        "::done_gone`",
        ":8: `src/l2_model/watchdog.rs::done_gone`: ",
    ),
    (
        "watchdog.rs::",
        "watchdogs.rs::",
        ":8: `src/l2_model/watchdogs.rs::done_progress`: no such",
    ),
    (
        "`::wait_progress`",
        "`::step`",
        ":8: `::step`: no file the row named before it",
    ),
    (
        "`Banner::Codex`",
        "`Banner::Claude`",
        ":8: `Banner::Claude`: neither",
    ),
    (
        "AddressGuard::check`,",
        "AddressGuard::gone`,",
        ":8: `tracked::AddressGuard::gone`",
    ),
    (
        "`run.rs::resumable`",
        "`run.rs::gone`",
        ":8: `run.rs::gone`: no src/…/run.rs declares",
    ),
    (
        "`run.rs::resumable`",
        "`nope.rs::resumable`",
        ":8: `nope.rs::resumable`: no src/",
    ),
    (
        "`end.rs`",
        "`ending.rs`",
        ":8: `ending.rs`: no src/…/ending.rs",
    ),
    (
        "`crate::run`",
        "`crate::gone`",
        ":8: `crate::gone`: neither",
    ),
    (
        "::pins_it",
        "::gone",
        ":9: `tests/it/floor.rs::gone`: tests/it/floor.rs declares no",
    ),
    (
        "`AddressGuard::check`)",
        "`AddressGuard::gone`)",
        ":9: `AddressGuard::gone`: neither",
    ),
    (
        "`src/l2_model/tracked.rs` (`AddressGuard::check`), pinned by `tests/it/floor.rs::pins_it`",
        "`ae doctor` only",
        ":9: the owner cell names no existing",
    ),
    (
        "rule | `src/l2_model/tracked.rs`",
        "rule | `::AddressGuard`",
        ":9: `::AddressGuard`: a continuation",
    ),
    ("`a \\| b`", "`a | b`", ":8: a row has 3 cells"),
    ("| Owner |", "| Owners |", ":6: the table header is not"),
    (
        "|---|---|\n",
        "",
        ":7: the header is not followed by a separator",
    ),
    (
        "## Key invariants",
        "## Invariants",
        "expected one `## Key invariants` heading, found 0",
    ),
    (
        "## Config",
        "## Key invariants",
        "expected one `## Key invariants` heading, found 2",
    ),
    (
        "\n## Config",
        "\n| Late | `src/lib.rs::run` |\n## Config",
        ":11: a table row after the table",
    ),
    (
        "`::wait_progress` its",
        "`::wait_progress its",
        ":8: an unclosed code span",
    ),
    (
        "`::wait_progress`",
        "`::wait progress`",
        ":8: `::wait progress`: a malformed item path",
    ),
    (
        "watchdog.rs::done_progress`",
        "watchdog.rs::`",
        ":8: `src/l2_model/watchdog.rs::`: a malformed",
    ),
    (
        "`run.rs::resumable`",
        "`run.r::resumable`",
        ":8: `run.r::resumable`: not an owner path",
    ),
    (
        "`end.rs`",
        "``end.rs``",
        ":8: an empty or double-backtick code span",
    ),
    (
        "`ae doctor`",
        "`ae::doctor x`",
        ":8: `ae::doctor x`: a malformed item path",
    ),
    (
        "worker}.txt`",
        "worker}.rs`",
        ":8: `src/l3_services/render/{core,worker}.rs`: a malformed file",
    ),
];

fn fixture() -> String {
    format!(
        "# Reference\n## Key invariants\n\nEach is one rule with one owner.\n\n\
         {INVARIANTS_HEADER}\n|---|---|\n{ROW_ONE}\n{ROW_TWO}\n\n## Config\n"
    )
}

fn fixture_known() -> Symbols {
    [
        ("src/lib.rs", "fn run() {}"),
        (
            "src/l2_model/watchdog.rs",
            "fn done_progress() {} fn wait_progress() {} enum Banner { Codex }",
        ),
        (
            "src/l2_model/tracked.rs",
            "struct AddressGuard; impl AddressGuard { fn check() {} }",
        ),
        ("src/l3_services/run.rs", "fn resumable() {}"),
        ("src/l5_ui/app/run.rs", "fn step() {}"),
        ("src/l4_orchestration/lifecycle/end.rs", "fn finish() {}"),
        ("tests/it/floor.rs", "#[test] fn pins_it() {}"),
    ]
    .into_iter()
    .map(|(path, text)| (path.to_owned(), rust_symbols(path, text)))
    .collect()
}

#[test]
fn the_key_invariant_owner_pin_rejects_planted_edits() {
    let known = fixture_known();
    let doc = fixture();
    // Row one has 7 references (the template path and `ae doctor` are prose),
    // row two 3: the count pins that the parser reads every one.
    assert_eq!(invariant_errors(&doc, &known), Ok((2, 10)));
    for (from, to, needle) in PLANTED {
        assert!(doc.contains(from), "the fixture lost {from}");
        let errors = invariant_errors(&doc.replacen(from, to, 1), &known)
            .err()
            .unwrap_or_default();
        assert!(
            errors.iter().any(|error| error.contains(needle)),
            "{from} -> {to}: wanted {needle:?}, got {errors:?}"
        );
    }
    let empty = doc.replace(ROW_ONE, "").replace(ROW_TWO, "");
    let errors = invariant_errors(&empty, &known).err().unwrap_or_default();
    assert!(
        errors.iter().any(|error| error.ends_with("has no rows")),
        "{errors:?}"
    );
    // Renaming the first owner of each form fails on exactly that owner.
    let first = first_of_each_form(&doc);
    assert_eq!(first.len(), 4, "{first:?}");
    for (form, (line, span)) in first {
        let errors = invariant_errors(&renamed_on(&doc, line, &span), &known)
            .err()
            .unwrap_or_default();
        assert_eq!(errors.len(), 1, "{form} {span}: {errors:?}");
        assert!(errors[0].starts_with(&format!("{REFERENCE}:{line}: `{span}_gone`")));
    }
}
