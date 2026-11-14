//! The LAYER MAP of `docs/reference/contract.md` names the ratchet's modules, layer by layer.
//!
//! [`MODULE_LAYERS`] in `layers.rs` is the one table of which top-level module of `src/`
//! belongs to which layer. The `### Layer map` subsection of the contract's Structure topic
//! is the one-screen reading aid over that table, so an agent learns the layers before it
//! opens a file. This guard keeps the two equal in both directions and prints the
//! difference: a module the doc lacks, a module the doc invents, a module the doc puts in
//! another layer, a layer whose name or item is wrong. The table wins; the doc line is what
//! gets fixed.
//!
//! Grammar of the doc, bounded on purpose. The subsection runs from the `### Layer map`
//! heading to the next heading. A layer is a list item `- **<n> <name>** ...`; the item
//! continues over the following lines up to a blank line or the next item. Its modules are
//! the backtick spans after the first `Modules:` in the item, and nothing else in the
//! subsection is read as a module.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::layers::{LAYER_NAMES, MODULE_LAYERS};

const REFERENCE: &str = "docs/reference/contract.md";
const HEADING: &str = "### Layer map";

fn read(path: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
        .unwrap_or_else(|error| panic!("{path}: {error}"))
}

/// One layer item of the doc: its number, its name, the modules it lists.
struct Item {
    layer: usize,
    name: String,
    modules: Vec<String>,
}

/// The lines of the `### Layer map` subsection, or why there are none.
fn subsection(doc: &str) -> Result<Vec<&str>, String> {
    let lines: Vec<&str> = doc.lines().collect();
    let at: Vec<usize> = (0..lines.len())
        .filter(|&index| lines[index].trim_end() == HEADING)
        .collect();
    match at.as_slice() {
        [start] => Ok(lines[start + 1..]
            .iter()
            .take_while(|line| !line.starts_with('#'))
            .copied()
            .collect()),
        [] => Err(format!("{REFERENCE} has no `{HEADING}` heading")),
        many => Err(format!(
            "{REFERENCE} has {} `{HEADING}` headings, want one",
            many.len()
        )),
    }
}

/// The backtick-quoted module names after `Modules:` in one item's text.
fn module_spans(text: &str) -> Result<Vec<String>, String> {
    let Some((_, after)) = text.split_once("Modules:") else {
        return Err("no `Modules:` in the item".to_owned());
    };
    let parts: Vec<&str> = after.split('`').collect();
    if parts.len().is_multiple_of(2) {
        return Err("an unclosed backtick after `Modules:`".to_owned());
    }
    let mut modules = Vec::new();
    for span in parts.iter().skip(1).step_by(2) {
        if span.is_empty()
            || !span
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(format!("`{span}` is not a module name"));
        }
        modules.push((*span).to_owned());
    }
    Ok(modules)
}

/// Every layer item of the subsection, plus the grammar errors met on the way.
fn items(lines: &[&str]) -> (Vec<Item>, Vec<String>) {
    let mut texts: Vec<String> = Vec::new();
    for line in lines {
        if line.starts_with("- **") {
            texts.push((*line).to_owned());
        } else if line.trim().is_empty() {
            texts.push(String::new());
        } else if let Some(last) = texts.last_mut()
            && !last.is_empty()
        {
            last.push(' ');
            last.push_str(line.trim());
        }
    }
    let mut parsed = Vec::new();
    let mut errors = Vec::new();
    for text in texts.iter().filter(|text| !text.is_empty()) {
        let Some((header, rest)) = text["- **".len()..].split_once("**") else {
            errors.push(format!("an item header is not closed: {text}"));
            continue;
        };
        let Some((number, name)) = header.split_once(' ') else {
            errors.push(format!("item header `{header}` wants `<n> <name>`"));
            continue;
        };
        let Ok(layer) = number.parse::<usize>() else {
            errors.push(format!(
                "item header `{header}`: `{number}` is not a layer number"
            ));
            continue;
        };
        match module_spans(rest) {
            Ok(modules) => parsed.push(Item {
                layer,
                name: name.to_owned(),
                modules,
            }),
            Err(why) => errors.push(format!("layer {layer} item: {why}")),
        }
    }
    (parsed, errors)
}

/// Every difference between the doc's layer map and the guard's table, one line each.
/// Empty means equal. `table` and `names` are the guard's own, passed in so a fixture can
/// stand in for them.
fn map_errors(doc: &str, table: &[(usize, &[&str])], names: &[&str]) -> Vec<String> {
    let lines = match subsection(doc) {
        Ok(lines) => lines,
        Err(why) => return vec![why],
    };
    let (parsed, mut errors) = items(&lines);

    let mut doc_items: BTreeMap<usize, &str> = BTreeMap::new();
    let mut doc_layers: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for item in &parsed {
        if doc_items.insert(item.layer, &item.name).is_some() {
            errors.push(format!("layer {} has two items in the doc", item.layer));
        }
        match names.get(item.layer) {
            None => errors.push(format!("layer {}: the guard has no such layer", item.layer)),
            Some(want) if *want != item.name => errors.push(format!(
                "layer {}: the doc names it `{}`, the guard names it `{want}`",
                item.layer, item.name
            )),
            Some(_) => {}
        }
        for module in &item.modules {
            doc_layers.entry(module).or_default().push(item.layer);
        }
    }

    let mut guard: BTreeMap<&str, usize> = BTreeMap::new();
    for (layer, modules) in table {
        if !doc_items.contains_key(layer) {
            errors.push(format!(
                "layer {layer} ({}): no `- **{layer} <name>**` item in the doc",
                names.get(*layer).copied().unwrap_or("?")
            ));
        }
        for module in *modules {
            guard.insert(module, *layer);
        }
    }

    for (module, layers) in &doc_layers {
        let unique: BTreeSet<_> = layers.iter().collect();
        if layers.len() > unique.len() || unique.len() > 1 {
            errors.push(format!(
                "`{module}` is listed more than once in the doc: layers {layers:?}"
            ));
        }
        for layer in unique {
            match guard.get(module) {
                None => errors.push(format!(
                    "`{module}`: the doc lists it in layer {layer}, the guard's table has no such module"
                )),
                Some(want) if want != layer => errors.push(format!(
                    "`{module}`: the doc lists it in layer {layer}, the guard's table puts it in layer {want}"
                )),
                Some(_) => {}
            }
        }
    }
    for (module, layer) in &guard {
        if !doc_layers.contains_key(module) {
            errors.push(format!(
                "`{module}`: the guard's table puts it in layer {layer}, the doc does not list it"
            ));
        }
    }
    errors
}

#[test]
fn the_layer_map_lists_exactly_the_modules_of_the_ratchet_table() {
    let errors = map_errors(&read(REFERENCE), &MODULE_LAYERS, &LAYER_NAMES);
    assert!(
        errors.is_empty(),
        "{} difference(s) between the `{HEADING}` of {REFERENCE} and MODULE_LAYERS in \
         tests/it/doors/layers.rs; the table is the source, fix the doc:\n{}",
        errors.len(),
        errors.join("\n")
    );
}

// ---- the grammar, on a doc rendered from the table ---------------------------------------

/// A doc that mirrors `table`: a Structure topic, the subsection with one item per layer
/// (the module list wrapped over continuation lines, as the real doc wraps it), and the
/// next topic. The shape the real doc has, built from the table so the two cannot drift
/// in this fixture.
fn rendered(table: &[(usize, &[&str])], names: &[&str]) -> String {
    let mut doc = String::from("## Structure\n\n```\nsrc/ - sources\n```\n\n");
    doc.push_str(HEADING);
    doc.push_str("\n\nA rule sentence that names no module.\n\n");
    for (layer, modules) in table {
        let mut line = format!("- **{layer} {}** - a purpose. Modules:", names[*layer]);
        for module in *modules {
            if line.len() > 84 {
                doc.push_str(&line);
                doc.push('\n');
                "  ".clone_into(&mut line);
            } else {
                line.push(' ');
            }
            line.push('`');
            line.push_str(module);
            line.push('`');
        }
        doc.push_str(&line);
        doc.push('\n');
    }
    doc.push_str("\nA closing sentence.\n\n## Session helpers\n\n- **9 stray** Modules: `json`\n");
    doc
}

fn fixture_table() -> Vec<(usize, &'static [&'static str])> {
    MODULE_LAYERS.to_vec()
}

fn fixture_errors(doc: &str) -> Vec<String> {
    map_errors(doc, &fixture_table(), &LAYER_NAMES)
}

fn good() -> String {
    rendered(&fixture_table(), &LAYER_NAMES)
}

#[test]
fn a_doc_that_mirrors_the_table_has_no_difference() {
    let doc = good();
    assert!(
        doc.matches("\n- **").count() >= LAYER_NAMES.len(),
        "the fixture must carry a wrapped item per layer"
    );
    assert!(
        doc.lines().any(|line| line.starts_with("  `")),
        "the fixture must wrap at least one module list over a continuation line"
    );
    assert_eq!(fixture_errors(&doc), Vec::<String>::new());
}

#[test]
fn a_module_the_doc_lacks_is_named() {
    let doc = good().replace(" `launch`", "");
    let errors = fixture_errors(&doc);
    assert!(
        errors
            .iter()
            .any(|line| line.contains("`launch`") && line.contains("layer 3")),
        "{errors:?}"
    );
}

#[test]
fn a_module_the_doc_invents_is_named() {
    let doc = good().replace(" `launch`", " `launch` `phantom`");
    let errors = fixture_errors(&doc);
    assert!(
        errors
            .iter()
            .any(|line| line.contains("`phantom`") && line.contains("no such module")),
        "{errors:?}"
    );
}

#[test]
fn a_module_the_doc_puts_in_another_layer_names_both_layers() {
    let doc = good()
        .replace(" `launch`", "")
        .replace("`console`", "`console` `launch`");
    let errors = fixture_errors(&doc);
    assert!(
        errors.iter().any(|line| line.contains("`launch`")
            && line.contains("doc lists it in layer 5")
            && line.contains("layer 3")),
        "{errors:?}"
    );
}

#[test]
fn a_module_listed_in_two_layers_is_named() {
    let doc = good().replace("`console`", "`console` `json`");
    let errors = fixture_errors(&doc);
    assert!(
        errors
            .iter()
            .any(|line| line.contains("`json`") && line.contains("more than once")),
        "{errors:?}"
    );
}

#[test]
fn a_wrong_layer_name_is_named() {
    let doc = good().replace("- **3 services**", "- **3 service**");
    let errors = fixture_errors(&doc);
    assert!(
        errors
            .iter()
            .any(|line| line.contains("layer 3") && line.contains("`service`")),
        "{errors:?}"
    );
}

#[test]
fn a_missing_layer_item_is_named_with_its_modules() {
    // The whole item goes: its header and the continuation lines that carry the modules.
    let doc = good();
    let (start, end) = (doc.find("- **1 ").unwrap(), doc.find("- **2 ").unwrap());
    let kept = format!("{}{}", &doc[..start], &doc[end..]);
    let errors = fixture_errors(&kept);
    assert!(
        errors
            .iter()
            .any(|line| line.contains("layer 1") && line.contains("no `- **1 <name>**` item")),
        "{errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|line| line.contains("`netprobe`") && line.contains("does not list it")),
        "{errors:?}"
    );
}

#[test]
fn a_missing_or_doubled_heading_is_refused() {
    let none = good().replace(HEADING, "### Something else");
    assert_eq!(
        fixture_errors(&none).len(),
        1,
        "{:?}",
        fixture_errors(&none)
    );
    let twice = format!("{}\n{HEADING}\n", good());
    assert_eq!(
        fixture_errors(&twice).len(),
        1,
        "{:?}",
        fixture_errors(&twice)
    );
}

#[test]
fn a_malformed_item_is_a_difference_not_a_skip() {
    let no_marker = good().replace(" Modules:", "");
    assert!(
        fixture_errors(&no_marker)
            .iter()
            .any(|line| line.contains("no `Modules:`")),
        "{:?}",
        fixture_errors(&no_marker)
    );
    let unclosed = good().replace("`json`", "`json");
    assert!(
        fixture_errors(&unclosed)
            .iter()
            .any(|line| line.contains("unclosed backtick") || line.contains("not a module name")),
        "{:?}",
        fixture_errors(&unclosed)
    );
    let bad_number = good().replace("- **2 ", "- **two ");
    assert!(
        fixture_errors(&bad_number)
            .iter()
            .any(|line| line.contains("not a layer number")),
        "{:?}",
        fixture_errors(&bad_number)
    );
}

#[test]
fn a_module_listed_twice_in_one_layer_is_named() {
    let doc = good().replace("`console`", "`console` `console`");
    let errors = fixture_errors(&doc);
    assert!(
        errors
            .iter()
            .any(|line| line.contains("`console`") && line.contains("more than once")),
        "{errors:?}"
    );
}

#[test]
fn a_doubled_or_unknown_layer_item_is_named() {
    let twice = good().replace(
        "\nA closing sentence.",
        "\n- **3 services** Modules: `launch`\n\nA closing sentence.",
    );
    assert!(
        fixture_errors(&twice)
            .iter()
            .any(|line| line.contains("layer 3") && line.contains("two items")),
        "{:?}",
        fixture_errors(&twice)
    );
    let unknown = good().replace(
        "\nA closing sentence.",
        "\n- **7 beyond** Modules: `phantom`\n\nA closing sentence.",
    );
    assert!(
        fixture_errors(&unknown)
            .iter()
            .any(|line| line.contains("layer 7") && line.contains("no such layer")),
        "{:?}",
        fixture_errors(&unknown)
    );
}
