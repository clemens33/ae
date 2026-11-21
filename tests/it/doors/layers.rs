//! The LAYER RATCHET — which layer of the crate a production file may name.
//!
//! Seven layer files and their folders hold the production modules. The ratchet reads
//! the source layer from the path and the destination layer of a root alias from the
//! layer file's production module declarations. The crate roots and the items of
//! src/lib.rs belong to the top layer. A file may name its own layer and the layers
//! below it. A `crate::` path that names a higher layer is an upward reference, with
//! today's spellings pinned in an allow-list that may only shrink.
//!
//! OWNERS. src/lib.rs declares the layer modules and re-exports their modules by name
//! (`pub use l0_base::json;`). The guard walks production registrations from the crate
//! roots. A layer file starts each chain, one layer owns each reached file, and the file
//! lies in its owner's folder. A layer file declares its modules as `pub mod` or
//! `pub(crate) mod`. Only src/lib.rs, the layer files and a `pub(in …)` scope may spell
//! `crate::l<N>_…`. Test-only files carry no owner. The crate roots and the seven layer
//! files are the only Rust files directly under src/.
//!
//! * [`layers_allow::DEBT`](super::layers_allow::DEBT) pins each spelling of an upward
//!   reference that is a defect to remove, as `(file, spelling, count)`.
//! * [`layers_allow::HANDSHAKE`](super::layers_allow::HANDSHAKE) pins the references that
//!   stay: the door layer takes typed argv values only their owner can build, so the door
//!   (who executes) and the owner (who interprets) stay apart on purpose. Each row names
//!   its seal. A handshake pins an exact spelling, never a whole module edge.
//!
//! The pin is EXACT in both directions. A spelling written more often than pinned (or not
//! pinned at all) is growth. A spelling written less often than pinned is slack: the
//! row must be lowered or deleted in the commit that removed the reference, so the list
//! can never silently allow a regrowth. A root item of `src/lib.rs` (`crate::state_root`)
//! counts as a layer 6 target, so every module that names one is upward.
//!
//! WHAT IT SEES. It reads the production text of a file as [`source_view`] defines it
//! (comments and `#[cfg(test)]` items blanked, so an import written after a test-only
//! item is still seen) and finds every `crate::` path spelling in its tokens: an
//! expression, a type, a `use` item with nested groups (`crate::{a, b::{C, self}}`), a
//! glob, and a path inside a macro call's arguments. A spelling is the path as written
//! up to a generic argument, a call or a field: `crate::config::Config::default`.
//! A raw identifier (`r#time`) names the module its plain spelling names, and its pin keeps
//! the spelling as written.
//!
//! WHAT IT DOES NOT SEE. It matches path SPELLINGS and resolves nothing: an alias
//! (`use crate::cli as c; c::Request`) names `crate::cli` once, at the `use`, and the
//! later `c::` uses are not seen; a re-export (`pub use`) hides where an item lives;
//! macro output (a macro that expands to a `crate::` path) is not expanded; an
//! unqualified path (`use super::x`, `self::x`, `$crate::x` inside a `macro_rules!`) is
//! not read. A module that reaches up only through one of those escapes this guard. It
//! catches a new upward spelling at the gate; it does not prove that no upward
//! dependency exists.
//!
use std::collections::{BTreeMap, BTreeSet, VecDeque, btree_map::Entry};
use std::path::Path;
use std::str::FromStr;
use std::sync::OnceLock;

use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};
use syn::ext::IdentExt as _;

use super::layers_allow::{DEBT, HANDSHAKE, Row};
use crate::support::source_view;

/// The layer names, bottom first. A file may name its own layer and the ones before it.
pub(super) const LAYER_NAMES: [&str; 7] = [
    "base",
    "platform doors",
    "state and model",
    "services",
    "seat and session orchestration",
    "terminal UI",
    "CLI shell",
];

/// The top layer: the CLI shell, which also holds `src/lib.rs`, `src/main.rs` and every
/// root item of `src/lib.rs`.
const TOP: usize = 6;

/// The layer modules, bottom first: `src/l0_base.rs` beside `src/l0_base/` holds layer 0.
pub(super) const LAYER_DIRS: [&str; 7] = [
    "l0_base",
    "l1_platform",
    "l2_model",
    "l3_services",
    "l4_orchestration",
    "l5_ui",
    "l6_shell",
];

/// The layer a layer-module name stands for: `l0_base` is 0.
fn layer_dir(name: &str) -> Option<usize> {
    LAYER_DIRS.iter().position(|dir| *dir == name)
}

/// The crate roots and their items belong to the top layer.
const ROOT_FILES: [&str; 2] = ["src/lib.rs", "src/main.rs"];

/// The layer file itself, as in `src/l0_base.rs`.
fn layer_file(file: &str) -> Option<usize> {
    layer_dir(file.strip_prefix("src/")?.strip_suffix(".rs")?)
}

/// A source file takes its layer from the folder or layer file at its path.
fn file_layer(file: &str) -> Result<usize, String> {
    if ROOT_FILES.contains(&file) {
        return Ok(TOP);
    }
    let folder = file
        .strip_prefix("src/")
        .and_then(|path| path.split_once('/'))
        .and_then(|(dir, _)| layer_dir(dir));
    folder.or_else(|| layer_file(file)).ok_or_else(|| {
        format!(
            "{file}: Rust source belongs in a layer folder, except for crate roots and layer files"
        )
    })
}

fn is_flat_rust(file: &str) -> bool {
    file.strip_prefix("src/").is_some_and(|name| {
        !name.contains('/') && Path::new(name).extension().is_some_and(|ext| ext == "rs")
    })
}

/// Layers of the production module names declared in the layer files.
pub(super) type ModuleLayers = BTreeMap<String, usize>;

/// The layer a `crate::<first>` path names: a layer module's own layer, a declared
/// module's layer, or the top layer for an item of src/lib.rs. A raw identifier
/// (`r#time`) looks up its plain spelling. The pin keeps the spelling as written.
fn target_layer(first: &str, modules: &ModuleLayers) -> usize {
    let plain = first.strip_prefix("r#").unwrap_or(first);
    layer_dir(plain)
        .or_else(|| modules.get(plain).copied())
        .unwrap_or(TOP)
}

// ---- the path spellings ------------------------------------------------------

/// One `crate::` path spelling found in a file's tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Spelling {
    /// The path as written, `crate::a::B`.
    text: String,
    /// The first segment after `crate::`: a module, or a root item of `src/lib.rs`.
    first: String,
    /// The source line of the path's first token.
    line: usize,
    /// Written inside a `pub(in …)` visibility, where it names a scope, not an item.
    visibility: bool,
}

fn is_punct(token: Option<&TokenTree>, ch: char, spacing: Option<Spacing>) -> bool {
    matches!(token, Some(TokenTree::Punct(punct))
        if punct.as_char() == ch && spacing.is_none_or(|want| punct.spacing() == want))
}

/// `tokens[at..]` starts with `::`.
fn is_sep(tokens: &[TokenTree], at: usize) -> bool {
    is_punct(tokens.get(at), ':', Some(Spacing::Joint)) && is_punct(tokens.get(at + 1), ':', None)
}

/// Read one use-tree or path that starts at `tokens[at]` (the segments after
/// `prefix`), push every leaf spelling and return the index after what it consumed.
fn read_tree(
    tokens: &[TokenTree],
    mut at: usize,
    prefix: &mut Vec<String>,
    out: &mut Vec<Spelling>,
) -> usize {
    let keep = prefix.len();
    let leaf_line = tokens.get(at).map_or(1, |token| token.span().start().line);
    loop {
        match tokens.get(at) {
            Some(TokenTree::Ident(ident)) => {
                let name = ident.to_string();
                at += 1;
                if is_sep(tokens, at) {
                    prefix.push(name);
                    at += 2;
                    continue;
                }
                if name != "self" {
                    prefix.push(name);
                }
                push_leaf(prefix, leaf_line, out);
                break;
            }
            Some(TokenTree::Punct(punct)) if punct.as_char() == '*' => {
                prefix.push("*".to_owned());
                push_leaf(prefix, leaf_line, out);
                at += 1;
                break;
            }
            Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Brace => {
                let items: Vec<TokenTree> = group.stream().into_iter().collect();
                for item in items.split(|token| is_punct(Some(token), ',', None)) {
                    // A trailing comma leaves an empty item that names nothing.
                    if !item.is_empty() {
                        read_tree(item, 0, prefix, out);
                    }
                }
                at += 1;
                break;
            }
            _ => {
                // A generic argument, a call, a field: the path ends where it stands.
                push_leaf(prefix, leaf_line, out);
                break;
            }
        }
    }
    prefix.truncate(keep);
    at
}

fn push_leaf(prefix: &[String], line: usize, out: &mut Vec<Spelling>) {
    // `crate` alone, or `crate::` with nothing after it, names no module.
    if let Some(first) = prefix.get(1)
        && prefix.last().is_some_and(|last| last != "crate")
    {
        out.push(Spelling {
            text: prefix.join("::"),
            first: first.clone(),
            line,
            visibility: false,
        });
    }
}

/// Every `crate::` path spelling in a token stream, nested groups included.
fn walk(stream: TokenStream, out: &mut Vec<Spelling>) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut at = 0;
    while at < tokens.len() {
        match &tokens[at] {
            TokenTree::Ident(ident)
                if ident == "crate"
                    && is_sep(&tokens, at + 1)
                    // `$crate` inside a `macro_rules!` body is not read (see the module doc).
                    && !(at > 0 && is_punct(tokens.get(at - 1), '$', None)) =>
            {
                at = read_tree(&tokens, at + 3, &mut vec!["crate".to_owned()], out);
            }
            TokenTree::Group(group) => {
                let start = out.len();
                walk(group.stream(), out);
                // `pub(in crate::a)` names the scope an item is visible in.
                let scope = group.delimiter() == Delimiter::Parenthesis
                    && at > 0
                    && matches!(&tokens[at - 1], TokenTree::Ident(ident) if ident == "pub")
                    && matches!(
                        group.stream().into_iter().next(),
                        Some(TokenTree::Ident(ident)) if ident == "in"
                    );
                if scope {
                    for spelling in &mut out[start..] {
                        spelling.visibility = true;
                    }
                }
                at += 1;
            }
            _ => at += 1,
        }
    }
}

fn spellings(file: &str, production: &str) -> Result<Vec<Spelling>, String> {
    let stream = TokenStream::from_str(production)
        .map_err(|why| format!("{file}: the production text does not lex: {why}"))?;
    let mut out = Vec::new();
    walk(stream, &mut out);
    Ok(out)
}

// ---- the check ---------------------------------------------------------------

/// One upward spelling of a file: how often it is written and where first.
struct Upward {
    count: usize,
    first_line: usize,
    to: usize,
}

/// Check a production file from its path's layer. Only src/lib.rs, the layer files and
/// a `pub(in …)` scope may spell a layer module directly. Each pin belongs to its file.
fn check_from(
    file: &str,
    from: usize,
    production: &str,
    allow: &[Row],
    modules: &ModuleLayers,
) -> Result<Vec<String>, String> {
    let canonical = file == "src/lib.rs" || layer_file(file).is_some();
    let mut direct = Vec::new();
    let mut upward: BTreeMap<String, Upward> = BTreeMap::new();
    for spelling in spellings(file, production)? {
        let plain = spelling
            .first
            .strip_prefix("r#")
            .unwrap_or(spelling.first.as_str());
        if !canonical && !spelling.visibility && layer_dir(plain).is_some() {
            direct.push(format!(
                "{file}:{}: `{}` spells the layer module `{plain}`; only src/lib.rs, the layer \
                 files and a `pub(in …)` scope may, so name the module through its root alias",
                spelling.line, spelling.text
            ));
        }
        let to = target_layer(&spelling.first, modules);
        if to > from {
            let entry = upward.entry(spelling.text).or_insert(Upward {
                count: 0,
                first_line: spelling.line,
                to,
            });
            entry.count += 1;
        }
    }
    let mut pinned: BTreeMap<&str, usize> = BTreeMap::new();
    for (row_file, text, count) in allow {
        if *row_file == file {
            *pinned.entry(*text).or_default() += *count;
        }
    }

    let mut findings = direct;
    for (text, found) in &upward {
        let pin = pinned.get(text.as_str()).copied().unwrap_or(0);
        if found.count > pin {
            findings.push(format!(
                "{file}:{}: upward `{text}` ({} -> {}) written {} time(s), pinned {pin}; \
                 the ratchet pins only today's upward spellings, so name a layer at or below \
                 {} instead",
                found.first_line,
                LAYER_NAMES[from],
                LAYER_NAMES[found.to],
                found.count,
                LAYER_NAMES[from],
            ));
        }
    }
    for (text, pin) in &pinned {
        let found = upward.get(*text);
        let count = found.map_or(0, |found| found.count);
        if count < *pin {
            findings.push(format!(
                "{file}:{}: slack: `{text}` is pinned {pin} time(s) and written {count}; lower \
                 or delete the row in tests/it/doors/layers_allow.rs (the allow-list only shrinks)",
                found.map_or(1, |found| found.first_line),
            ));
        }
    }
    Ok(findings)
}

/// Classify raw source by the shared view, then check its production text.
fn classified(file: &str, source: &str, allow: &[Row]) -> Result<Vec<String>, String> {
    // The layer first: a file without one is refused before its text is read.
    let from = file_layer(file)?;
    let class = source_view::classify(file, source).map_err(|failure| failure.to_string())?;
    check_from(file, from, &class.production, allow, real_module_layers())
}

/// The pinned rows of the real tree: the debt and the permanent handshakes.
fn real_rows() -> Vec<Row> {
    DEBT.iter()
        .copied()
        .chain(
            HANDSHAKE
                .iter()
                .map(|(file, text, count, _seal)| (*file, *text, *count)),
        )
        .collect()
}

/// The findings of one file's raw source against the real allow-list. `Err` is a file
/// the guard cannot judge: no reviewed layer, or source the shared view refuses.
pub(super) fn violations(file: &str, source: &str) -> Result<Vec<String>, String> {
    classified(file, source, &real_rows())
}

/// [`violations`] against an explicit allow-list.
pub(super) fn violations_with(
    file: &str,
    source: &str,
    allow: &[Row],
) -> Result<Vec<String>, String> {
    classified(file, source, allow)
}

/// Check a whole source tree: its paths, production declarations, owners and spellings.
/// The real-tree guard and frozen fixtures both enter here. A source-view failure is
/// an error. A policy violation is a finding.
pub(super) fn violations_in(
    files: &[(String, String)],
    allow: &[Row],
) -> Result<Vec<String>, String> {
    // Inspect paths before reachability so a gated or unregistered flat file is refused.
    // Return layout findings first, then check declarations and imports after the paths are fixed.
    let mut findings: Vec<_> = files
        .iter()
        .filter(|(path, _)| is_flat_rust(path))
        .filter_map(|(path, _)| file_layer(path).err())
        .collect();
    let has_flat = !findings.is_empty();
    for (path, text) in files
        .iter()
        .filter(|(path, _)| ROOT_FILES.contains(&path.as_str()))
    {
        let class = source_view::classify(path, text).map_err(|failure| failure.to_string())?;
        findings.extend(root_declaration_findings(path, &class.production)?);
    }
    if has_flat {
        return Ok(findings);
    }

    let product = source_view::product_of(files).map_err(|failure| failure.to_string())?;
    let layer_sources = files.iter().filter_map(|(path, source)| {
        let layer = layer_file(path)?;
        let file = product.files.iter().find(|file| file.path == *path)?;
        Some((layer, path.as_str(), source.as_str(), file.text.as_str()))
    });
    let (modules, declaration_findings) = layer_declarations(layer_sources)?;
    findings.extend(declaration_findings);
    if let Some(lib) = product.files.iter().find(|file| file.path == "src/lib.rs") {
        findings.extend(alias_findings(&root_uses(&lib.text)?, &modules));
    }
    let mut classified = BTreeMap::new();
    for (path, text) in files {
        let class = source_view::classify(path, text).map_err(|failure| failure.to_string())?;
        classified.insert(path.as_str(), class);
    }
    findings.extend(owner_findings(&classified)?);
    for file in product
        .files
        .iter()
        .filter(|file| file.path.starts_with("src/"))
    {
        match file_layer(&file.path) {
            Ok(from) => findings.extend(check_from(&file.path, from, &file.text, allow, &modules)?),
            Err(why) => findings.push(why),
        }
    }
    Ok(findings)
}

// ---- the owner walk (D4) -----------------------------------------------------

fn owner_description(layer: usize) -> String {
    format!(
        "layer {layer} ({}) through src/{}.rs",
        LAYER_NAMES[layer], LAYER_DIRS[layer]
    )
}

/// One ungated registration the walk follows: the file it names, the owner it carries, its
/// `file:line`, the module name, and whether a crate root wrote it.
struct Edge {
    target: String,
    layer: usize,
    site: String,
    name: String,
    from_root: bool,
}

/// The file an ungated registration names, as the shared view resolves it.
fn src_target(
    file: &str,
    reg: &source_view::Registration,
    known: &[&str],
) -> Result<String, String> {
    match source_view::target_of(file, reg, known) {
        Ok(source_view::Target::Src(target)) => Ok(target),
        Ok(source_view::Target::Outside(target)) => Err(format!(
            "{file}:{}: `mod {}` registers {target}, outside src/",
            reg.line, reg.name
        )),
        Err(failure) => Err(failure.to_string()),
    }
}

/// Follow the ungated production registrations. Each chain starts with a layer file
/// registered by src/lib.rs. Every later file must lie in that layer's folder.
fn owner_findings(
    classified: &BTreeMap<&str, source_view::Classified>,
) -> Result<Vec<String>, String> {
    let known: Vec<&str> = classified.keys().copied().collect();
    let mut owners: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut findings = Vec::new();
    let mut queue = VecDeque::new();
    for root in ROOT_FILES {
        let Some(class) = classified.get(root) else {
            continue;
        };
        for reg in class.registrations.iter().filter(|reg| !reg.gated) {
            let site = format!("{root}:{}", reg.line);
            if !reg.nesting.is_empty() {
                findings.push(format!(
                    "{site}: `mod {}` must be declared inside a layer folder",
                    reg.name
                ));
            } else if root == "src/lib.rs"
                && let Some(layer) = layer_dir(reg.name.strip_prefix("r#").unwrap_or(&reg.name))
            {
                queue.push_back(Edge {
                    target: src_target(root, reg, &known)?,
                    layer,
                    site,
                    name: reg.name.clone(),
                    from_root: true,
                });
            }
        }
    }

    while let Some(Edge {
        target,
        layer,
        site,
        name,
        from_root,
    }) = queue.pop_front()
    {
        let dir = LAYER_DIRS[layer];
        let at_home = if from_root {
            target == format!("src/{dir}.rs")
        } else {
            target.starts_with(&format!("src/{dir}/"))
        };
        if !at_home {
            findings.push(format!(
                "{site}: `mod {name}` places {target} outside its owner's layer, {}",
                owner_description(layer)
            ));
        }
        if let Some((first, first_site)) = owners.get(&target) {
            if *first != layer {
                findings.push(format!(
                    "{target}: two owners reach it, {first_site} ({}) and {site} ({}); a \
                     production file has one owner",
                    owner_description(*first),
                    owner_description(layer)
                ));
            }
            continue;
        }
        let Some(class) = classified.get(target.as_str()) else {
            return Err(format!("{site}: registration target {target} is missing"));
        };
        for reg in class.registrations.iter().filter(|reg| !reg.gated) {
            queue.push_back(Edge {
                target: src_target(&target, reg, &known)?,
                layer,
                site: format!("{target}:{}", reg.line),
                name: reg.name.clone(),
                from_root: false,
            });
        }
        owners.insert(target, (layer, site));
    }
    Ok(findings)
}

// ---- the declarations (D1) and the root aliases (D4) -------------------------

/// The visibility of an item as written: `pub`, `pub(crate)`, `pub(in a::b)`, or empty.
fn visibility(vis: &syn::Visibility) -> String {
    match vis {
        syn::Visibility::Public(_) => "pub".to_owned(),
        syn::Visibility::Restricted(restricted) => {
            let path = restricted
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>()
                .join("::");
            if restricted.in_token.is_some() {
                format!("pub(in {path})")
            } else {
                format!("pub({path})")
            }
        }
        syn::Visibility::Inherited => String::new(),
    }
}

/// The file-module declarations of a file: `(visibility, name, line)`.
fn mod_declarations(file: &str, text: &str) -> Result<Vec<(String, String, usize)>, String> {
    let tree = syn::parse_file(text)
        .map_err(|why| format!("{file}: the declarations do not parse: {why}"))?;
    Ok(tree
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Mod(module) if module.content.is_none() => Some((
                visibility(&module.vis),
                module.ident.unraw().to_string(),
                module.mod_token.span.start().line,
            )),
            _ => None,
        })
        .collect())
}

/// One leaf of a top-level `use` of src/lib.rs: its path, the line of each segment, the name it
/// binds, and whether it is a glob. At the crate root a leading `crate::` or `self::` names the
/// root, so it is dropped, and every identifier is in its plain spelling (`r#l5_ui` is `l5_ui`).
/// Both callers, the alias rule and the door resolver, therefore see one spelling per path.
struct UseLeaf {
    path: Vec<(String, usize)>,
    binds: String,
    glob: bool,
}

fn use_leaves(tree: &syn::UseTree, prefix: &mut Vec<(String, usize)>, out: &mut Vec<UseLeaf>) {
    let line = |ident: &proc_macro2::Ident| ident.span().start().line;
    let plain = |ident: &proc_macro2::Ident| ident.unraw().to_string();
    let leaf = |prefix: &[(String, usize)], ident: &proc_macro2::Ident, binds: String| {
        let mut path = prefix.to_vec();
        if ident != "self" {
            path.push((plain(ident), line(ident)));
        }
        UseLeaf {
            path,
            binds,
            glob: false,
        }
    };
    match tree {
        syn::UseTree::Path(step) => {
            if prefix.is_empty() && (step.ident == "crate" || step.ident == "self") {
                use_leaves(&step.tree, prefix, out);
            } else {
                prefix.push((plain(&step.ident), line(&step.ident)));
                use_leaves(&step.tree, prefix, out);
                prefix.pop();
            }
        }
        syn::UseTree::Name(name) => {
            let binds = if name.ident == "self" {
                prefix
                    .last()
                    .map(|(last, _)| last.clone())
                    .unwrap_or_default()
            } else {
                plain(&name.ident)
            };
            out.push(leaf(prefix, &name.ident, binds));
        }
        syn::UseTree::Rename(rename) => {
            out.push(leaf(prefix, &rename.ident, plain(&rename.rename)));
        }
        syn::UseTree::Glob(_) => out.push(UseLeaf {
            path: prefix.clone(),
            binds: String::new(),
            glob: true,
        }),
        syn::UseTree::Group(group) => {
            for item in &group.items {
                use_leaves(item, prefix, out);
            }
        }
    }
}

/// Every leaf of the top-level `use` items of src/lib.rs.
fn root_uses(lib: &str) -> Result<Vec<UseLeaf>, String> {
    let tree = syn::parse_file(lib)
        .map_err(|why| format!("src/lib.rs: the declarations do not parse: {why}"))?;
    let mut out = Vec::new();
    for item in &tree.items {
        if let syn::Item::Use(used) = item {
            use_leaves(&used.tree, &mut Vec::new(), &mut out);
        }
    }
    Ok(out)
}

/// A crate root's production file declarations may only introduce layer modules in lib.rs.
fn root_declaration_findings(file: &str, production: &str) -> Result<Vec<String>, String> {
    let mut findings = Vec::new();
    for (vis, name, line) in mod_declarations(file, production)? {
        if file == "src/lib.rs" && vis.is_empty() && layer_dir(&name).is_some() {
            continue;
        }
        let prefix = if vis.is_empty() {
            String::new()
        } else {
            format!("{vis} ")
        };
        findings.push(format!(
            "{file}:{line}: `{prefix}mod {name};` at a crate root; only src/lib.rs declares \
             private layer modules"
        ));
    }
    Ok(findings)
}

/// Check the visibility of all declarations, then map the production names to their layers.
fn layer_declarations<'a>(
    files: impl IntoIterator<Item = (usize, &'a str, &'a str, &'a str)>,
) -> Result<(ModuleLayers, Vec<String>), String> {
    let mut declarations = BTreeMap::new();
    let mut findings = Vec::new();
    for (layer, file, source, production) in files {
        for (vis, name, line) in mod_declarations(file, source)? {
            match vis.as_str() {
                "pub" | "pub(crate)" => {}
                "" => findings.push(format!(
                    "{file}:{line}: private `mod {name};` in a layer file, which declares a \
                     module only as `pub mod` or `pub(crate) mod` (D1)"
                )),
                _ => findings.push(format!(
                    "{file}:{line}: `{vis} mod {name};` in a layer file, which declares a module \
                     only as `pub mod` or `pub(crate) mod` (D1)"
                )),
            }
        }
        for (_, name, line) in mod_declarations(file, production)? {
            match declarations.entry(name) {
                Entry::Vacant(entry) => {
                    entry.insert((layer, file, line));
                }
                Entry::Occupied(entry) => {
                    let (_, first, first_line) = entry.get();
                    findings.push(format!(
                        "{file}:{line}: module `{}` is also declared at {first}:{first_line}",
                        entry.key()
                    ));
                }
            }
        }
    }
    let modules = declarations
        .into_iter()
        .map(|(name, (layer, _, _))| (name, layer))
        .collect();
    Ok((modules, findings))
}

/// The map for the real documentation and the fixtures that supply a single source file.
pub(super) fn real_module_layers() -> &'static ModuleLayers {
    static MAP: OnceLock<ModuleLayers> = OnceLock::new();
    MAP.get_or_init(|| {
        let files: Vec<_> = source_view::tree()
            .iter()
            .filter_map(|(file, text)| {
                let layer = layer_file(file)?;
                let class = source_view::classify(file, text).unwrap_or_else(|why| panic!("{why}"));
                Some((layer, file.as_str(), text.as_str(), class.production))
            })
            .collect();
        let (modules, findings) =
            layer_declarations(files.iter().map(|(layer, file, source, production)| {
                (*layer, *file, *source, production.as_str())
            }))
            .unwrap_or_else(|why| panic!("{why}"));
        assert!(findings.is_empty(), "{}", findings.join("\n"));
        modules
    })
}

/// Each root alias through a layer module must name a module that layer declares
/// and bind it under its declared name.
fn alias_findings(leaves: &[UseLeaf], modules: &ModuleLayers) -> Vec<String> {
    let mut findings = Vec::new();
    for leaf in leaves {
        let Some((first, line)) = leaf.path.first() else {
            continue;
        };
        let Some(layer) = layer_dir(first) else {
            continue;
        };
        let dir = LAYER_DIRS[layer];
        let spelled = leaf
            .path
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join("::");
        match leaf.path.as_slice() {
            _ if leaf.glob => findings.push(format!(
                "src/lib.rs:{line}: a glob re-export from {dir}; re-export each module by name"
            )),
            [_, (module, line)] => {
                if modules.get(module) != Some(&layer) {
                    findings.push(format!(
                        "src/lib.rs:{line}: the root alias `{dir}::{module}` names no module \
                         that src/{dir}.rs registers"
                    ));
                }
                if leaf.binds != *module {
                    findings.push(format!(
                        "src/lib.rs:{line}: the root alias `{spelled}` binds `{}`; D1 \
                         re-exports a layer module only under its own name",
                        leaf.binds
                    ));
                }
            }
            [_, (module, line), ..] => findings.push(format!(
                "src/lib.rs:{line}: the root `use` of `{spelled}` binds `{}` below the layer \
                 module `{dir}::{module}`; D1 re-exports a layer module only under its own name",
                leaf.binds
            )),
            _ => findings.push(format!(
                "src/lib.rs:{line}: the root `use` of `{spelled}` binds `{}` to the layer module \
                 itself; D1 re-exports each of its modules under its own name",
                leaf.binds
            )),
        }
    }
    findings
}

// ---- module paths (the door citations) ---------------------------------------

/// The file of library module `path` (`["app", "run"]`) among `files`: through the `mod`
/// registrations from src/lib.rs, where a production root `use` (`pub use l0_base::json;`)
/// stands for the module it names; a `#[cfg(test)]` root `use` names no door. `Err` names the
/// first segment that does not resolve to one file.
pub(super) fn module_file(files: &[(String, String)], path: &[&str]) -> Result<String, String> {
    resolve_module(files, path, 0)
}

fn resolve_module(
    files: &[(String, String)],
    path: &[&str],
    depth: usize,
) -> Result<String, String> {
    let known: Vec<&str> = files.iter().map(|(file, _)| file.as_str()).collect();
    let text_of = |file: &str| {
        files
            .iter()
            .find(|(name, _)| name == file)
            .map(|(_, text)| text.as_str())
            .ok_or_else(|| format!("{file} is not among the files"))
    };
    let joined = path.join("::");
    if depth > 8 {
        return Err(format!("`{joined}` goes through more than 8 root aliases"));
    }
    let mut file = "src/lib.rs".to_owned();
    for (at, segment) in path.iter().enumerate() {
        let class =
            source_view::classify(&file, text_of(&file)?).map_err(|failure| failure.to_string())?;
        let named =
            |reg: &&source_view::Registration| reg.name == *segment && reg.nesting.is_empty();
        let regs: Vec<_> = class
            .registrations
            .iter()
            .filter(named)
            .filter(|reg| !reg.gated)
            .collect();
        if regs.is_empty()
            && class
                .registrations
                .iter()
                .filter(named)
                .any(|reg| reg.gated)
        {
            return Err(format!(
                "`{segment}` of `{joined}` is registered in {file} only for tests, so it is no door"
            ));
        }
        let aliases: Vec<Vec<String>> = if at == 0 {
            root_uses(&class.production)?
                .into_iter()
                .filter(|leaf| !leaf.glob && leaf.binds == *segment)
                .map(|leaf| leaf.path.into_iter().map(|(name, _)| name).collect())
                .collect()
        } else {
            Vec::new()
        };
        file = match (regs.as_slice(), aliases.as_slice()) {
            ([reg], []) => src_target(&file, reg, &known)?,
            ([], [alias]) => {
                let alias: Vec<&str> = alias.iter().map(String::as_str).collect();
                resolve_module(files, &alias, depth + 1)?
            }
            ([], []) => {
                return Err(format!("`{segment}` of `{joined}` is no module of {file}"));
            }
            _ => {
                return Err(format!(
                    "`{segment}` of `{joined}` is ambiguous in {file}: {} registration(s) and {} \
                     root `use`(s) bind it",
                    regs.len(),
                    aliases.len()
                ));
            }
        };
    }
    Ok(file)
}

/// Whether module file `file` declares a top-level function `function` in its production view:
/// comments and `#[cfg(test)]` items do not count, and neither does a function inside an inline
/// module, an `impl` or another function.
pub(super) fn declares_function(file: &str, text: &str, function: &str) -> Result<bool, String> {
    let production = source_view::classify(file, text)
        .map_err(|failure| failure.to_string())?
        .production;
    let parsed =
        syn::parse_file(&production).map_err(|why| format!("{file} does not parse: {why}"))?;
    Ok(parsed
        .items
        .iter()
        .any(|item| matches!(item, syn::Item::Fn(found) if found.sig.ident == function)))
}

// ---- the real tree -----------------------------------------------------------

/// Every `.rs` file of this crate's `src/`, as `(path, raw text)`.
fn src_files() -> Vec<(String, String)> {
    source_view::tree()
        .iter()
        .filter(|(path, _)| path.starts_with("src/"))
        .cloned()
        .collect()
}

/// The `src/` files production code reaches, as `(path, production text)`.
fn product_files() -> Vec<(&'static str, &'static str)> {
    source_view::product()
        .files
        .iter()
        .filter(|file| file.path.starts_with("src/"))
        .map(|file| (file.path.as_str(), file.text.as_str()))
        .collect()
}

#[test]
fn no_production_file_names_an_unpinned_higher_layer() {
    let findings = violations_in(&src_files(), &real_rows()).unwrap_or_else(|why| panic!("{why}"));
    assert!(
        findings.is_empty(),
        "{} layer finding(s):\n{}",
        findings.len(),
        findings.join("\n")
    );
}

#[test]
fn every_pin_is_unique_positive_and_names_a_production_file() {
    let produced: BTreeSet<&str> = product_files().iter().map(|(path, _)| *path).collect();
    let mut seen = BTreeSet::new();
    for (file, text, count) in real_rows() {
        assert!(
            count > 0,
            "{file}: `{text}` is pinned zero times: delete the row"
        );
        assert!(
            text.starts_with("crate::"),
            "{file}: `{text}` is not a crate:: spelling"
        );
        assert!(
            produced.contains(file),
            "{file}: a pin names no production file (a moved or deleted file leaves its rows behind)"
        );
        assert!(
            seen.insert((file, text)),
            "{file}: `{text}` is pinned twice (debt and handshake rows share one key space)"
        );
    }
}

#[test]
fn a_handshake_is_a_door_layer_seal_with_a_named_reason() {
    for (file, text, _, seal) in HANDSHAKE {
        assert!(!seal.trim().is_empty(), "{file}: `{text}` names no seal");
        assert_eq!(
            file_layer(file),
            Ok(1),
            "{file}: a handshake is a typed argv seal of the door layer (platform doors)"
        );
    }
}

// ---- the spellings the frozen acceptance does not pin ------------------------

/// Every spelling of `source`, in source order.
fn spelled(source: &str) -> Vec<String> {
    spellings("src/x.rs", source)
        .unwrap_or_else(|why| panic!("{why}"))
        .into_iter()
        .map(|found| found.text)
        .collect()
}

#[test]
fn a_spelling_is_the_path_as_written_up_to_its_first_non_path_token() {
    assert_eq!(
        spelled("fn f() { crate::config::Config::default(); }"),
        ["crate::config::Config::default"]
    );
    assert_eq!(
        spelled("fn f() -> Vec<crate::meta::Meta> { crate::a::b::<crate::c::D>(); }"),
        ["crate::meta::Meta", "crate::a::b", "crate::c::D"]
    );
    assert_eq!(
        spelled("fn f() { let _ = crate::a::B { x: 1 }; }"),
        ["crate::a::B"]
    );
}

#[test]
fn a_use_tree_expands_to_one_spelling_per_leaf() {
    assert_eq!(
        spelled("use crate::{a::{B, C as D, self}, e, f::*};"),
        [
            "crate::a::B",
            "crate::a::C",
            "crate::a",
            "crate::e",
            "crate::f::*"
        ]
    );
}

#[test]
fn a_visibility_a_dollar_crate_and_a_bare_crate_are_not_spellings() {
    assert_eq!(
        spelled("pub(crate) fn f() {} pub(crate) struct S; fn g(_: crate::A) {}"),
        ["crate::A"]
    );
    assert!(spelled("macro_rules! m { () => { $crate::a::B } }").is_empty());
    assert!(spelled("use crate as me; extern crate alloc;").is_empty());
    assert_eq!(
        spelled("use crate::a::{B, C,};"),
        ["crate::a::B", "crate::a::C"]
    );
}

#[test]
fn a_spelling_names_the_line_of_its_own_token() {
    let found = spellings(
        "src/x.rs",
        "fn f() {}\nuse crate::{\n    a::B,\n    c::D,\n};",
    )
    .unwrap_or_else(|why| panic!("{why}"));
    let lines: Vec<(String, usize)> = found.into_iter().map(|s| (s.text, s.line)).collect();
    assert_eq!(
        lines,
        [("crate::a::B".to_owned(), 3), ("crate::c::D".to_owned(), 4)]
    );
}

/// A fixture tree for the driver regression cases, its paths assembled from pieces.
fn fixture_tree(files: &[(&str, &str)]) -> Vec<(String, String)> {
    files
        .iter()
        .map(|(module, text)| (format!("{}/{module}.rs", "src"), (*text).to_owned()))
        .collect()
}

/// The guard places the fixture tree and reports a finding at `site` that names every piece
/// of `evidence`.
fn p7_review_refuses(files: &[(&str, &str)], site: &str, evidence: &[&str]) {
    let found = violations_in(&fixture_tree(files), &[]).unwrap_or_else(|why| panic!("{why}"));
    assert!(
        found.iter().any(|finding| {
            finding.contains(site) && evidence.iter().all(|part| finding.contains(part))
        }),
        "expected a finding at {site} naming {evidence:?}, got {found:#?}"
    );
}

#[test]
fn p7_review_a_test_only_flat_file_is_refused() {
    // Review r1 (layoutp7-rev): the gated registration makes src/fixture.rs test-only, so
    // production reachability never shows it; only the path check refuses it.
    let path = format!("{}/fixture.rs", "src");
    p7_review_refuses(
        &[
            (
                "lib",
                "mod l0_base;\npub use l0_base::json;\n#[cfg(test)]\nmod fixture;\n",
            ),
            ("l0_base", "pub mod json;\n"),
            ("l0_base/json", ""),
            ("fixture", ""),
        ],
        &path,
        &[],
    );
}

#[test]
fn p7_review_src_lib_rs_declares_its_layer_modules_privately() {
    let site = format!("{}/lib.rs:1", "src");
    p7_review_refuses(
        &[
            ("lib", "pub mod l0_base;\npub use l0_base::json;\n"),
            ("l0_base", "pub mod json;\n"),
            ("l0_base/json", ""),
        ],
        &site,
        &["l0_base"],
    );
}

#[test]
fn p7_review_src_main_rs_declares_no_module() {
    let site = format!("{}/main.rs:1", "src");
    p7_review_refuses(
        &[
            ("lib", "mod l0_base;\npub use l0_base::json;\n"),
            ("main", "mod l0_base;\nfn main() {}\n"),
            ("l0_base", "pub mod json;\n"),
            ("l0_base/json", ""),
        ],
        &site,
        &["l0_base"],
    );
}

#[test]
fn p7_review_a_test_only_layer_file_declaration_keeps_the_visibility_rule() {
    // Review r2 (layoutp7-rev): a gated declaration stays out of the destination map, but a
    // layer file still declares every module as `pub mod` or `pub(crate) mod`.
    let site = format!("{}/l0_base.rs:3", "src");
    p7_review_refuses(
        &[
            ("lib", "mod l0_base;\npub use l0_base::json;\n"),
            ("l0_base", "pub mod json;\n#[cfg(test)]\nmod fixture;\n"),
            ("l0_base/json", ""),
            ("l0_base/fixture", ""),
        ],
        &site,
        &["private", "fixture"],
    );
}

#[test]
fn p0_review_a_root_alias_through_a_layer_module_binds_the_module_name() {
    // Review r2 (layout-nav): `console` is registered under its own name, and only the root
    // binding changes. `crate::time` would read as layer 0 while it reaches layer 5.
    let lib = |alias: &str| format!("mod l5_ui;\n{alias}\n");
    let tree = |alias: &str| {
        let lib = lib(alias);
        fixture_tree(&[
            ("lib", lib.as_str()),
            ("l5_ui", "pub mod console;\n"),
            ("l5_ui/console", "pub fn shown() {}\n"),
        ])
    };
    for alias in [
        "pub use l5_ui::console as time;",
        "pub use l5_ui::{console as time};",
        "pub use l5_ui::console::{self as time};",
        "pub use l5_ui::console::shown as time;",
    ] {
        let found = violations_in(&tree(alias), &[]).unwrap_or_else(|why| panic!("{why}"));
        assert!(
            found
                .iter()
                .any(|finding| finding.contains("l5_ui::console") && finding.contains("`time`")),
            "{alias}: {found:?}"
        );
    }
    // The layer module itself under another name hands out the same reach.
    let alias = "pub(crate) use l5_ui as time;";
    let found = violations_in(&tree(alias), &[]).unwrap_or_else(|why| panic!("{why}"));
    assert!(
        found
            .iter()
            .any(|finding| finding.contains("`l5_ui`") && finding.contains("`time`")),
        "{alias}: {found:?}"
    );
    // Control: the same tree, re-exported under the module's own name.
    assert_eq!(
        violations_in(&tree("pub use l5_ui::console;"), &[]),
        Ok(Vec::new())
    );
}

#[test]
fn p0_review_a_root_alias_is_read_in_its_plain_crate_relative_spelling() {
    // Review r3 (layout-nav): at the crate root `self::` and `crate::` name the root, and
    // `r#l5_ui` names `l5_ui`. A rename spelled either way is still a rename.
    let tree = |alias: &str| {
        let lib = format!("mod l5_ui;\n{alias}\n");
        fixture_tree(&[
            ("lib", lib.as_str()),
            ("l5_ui", "pub mod console;\n"),
            ("l5_ui/console", "pub fn shown() {}\n"),
        ])
    };
    for alias in [
        "pub use self::l5_ui::console as time;",
        "pub use crate::l5_ui::console as time;",
        "pub use r#l5_ui::console as time;",
        "pub use r#l5_ui::r#console as r#time;",
        "pub use self::{l5_ui::console as time};",
    ] {
        let found = violations_in(&tree(alias), &[]).unwrap_or_else(|why| panic!("{why}"));
        assert!(
            found
                .iter()
                .any(|finding| finding.contains("l5_ui::console") && finding.contains("`time`")),
            "{alias}: {found:?}"
        );
    }
    // Controls: the same-name alias in each spelling is accepted.
    for alias in [
        "pub use self::l5_ui::console;",
        "pub use crate::l5_ui::console;",
        "pub use r#l5_ui::r#console;",
    ] {
        assert_eq!(violations_in(&tree(alias), &[]), Ok(Vec::new()), "{alias}");
    }
}

#[test]
fn a_raw_identifier_names_the_same_module_as_its_plain_spelling() {
    let found = |file: &str, source: &str, allow: &[Row]| {
        violations_with(file, source, allow).unwrap_or_else(|why| panic!("{why}"))
    };
    // `r#time` is the module `time`: sideways from a layer 0 file and downward from the CLI
    // shell, so neither is a finding.
    assert!(found("src/l0_base/json.rs", "use crate::r#time;", &[]).is_empty());
    assert!(found("src/l6_shell/cli.rs", "use crate::r#time::Stamp;", &[]).is_empty());
    // `r#cli` is the module `cli`: upward from layer 0, and the pin keys on the spelling as
    // written, so the plain spelling's row does not admit it and its own row does.
    let up = found("src/l0_base/json.rs", "use crate::r#cli::Request;", &[]);
    assert_eq!(up.len(), 1, "{up:?}");
    assert!(up[0].contains("`crate::r#cli::Request`"), "{up:?}");
    let plain = [("src/l0_base/json.rs", "crate::cli::Request", 1)];
    assert!(!found("src/l0_base/json.rs", "use crate::r#cli::Request;", &plain).is_empty());
    let raw = [("src/l0_base/json.rs", "crate::r#cli::Request", 1)];
    assert!(found("src/l0_base/json.rs", "use crate::r#cli::Request;", &raw).is_empty());
    // A raw root item is still a root item: layer 6.
    assert!(!found("src/l0_base/json.rs", "fn live() { crate::r#type(); }", &[]).is_empty());
}
