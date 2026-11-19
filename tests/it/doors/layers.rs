//! The LAYER RATCHET — which layer of the crate a production file may name.
//!
//! Every top-level module of `src/` belongs to one of seven layers (base, platform doors,
//! state and model, services, seat and session orchestration, terminal UI, CLI shell); a
//! file belongs to the layer of its top-level module, `src/lib.rs` and `src/main.rs` to
//! the top one. A file may name its own layer and the layers below it. A `crate::` path
//! that names a HIGHER layer is an UPWARD reference. Most of them are old and most are
//! small (a predicate or constant that lives one layer too high), so the guard does not
//! forbid them: it PINS today's upward spellings in an allow-list that may only shrink.
//!
//! OWNERS. A layer may live in a folder: `src/l0_base.rs` beside `src/l0_base/` holds
//! layer 0, src/lib.rs declares the layer module private and re-exports each module by name
//! (`pub use l0_base::json;`). The guard walks the registrations from the crate roots and
//! gives every production file one owner: the layer file that starts its chain, or the
//! table's layer of its flat top-level module. A file lies in its owner's folder (a flat
//! owner's file in none), one owner reaches it, and the table agrees with the owner. The
//! declarations keep one spelling each: src/lib.rs writes `pub mod` or `mod`, a layer file
//! `pub mod` or `pub(crate) mod`, and only src/lib.rs, the layer files and a `pub(in …)`
//! scope spell `crate::l<N>_…`. Test-only files carry no owner.
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
//! A module the table does not name has no reviewed layer, and the guard refuses the
//! file instead of picking one. Assigning a layer to a new module is a review.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

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

/// The reviewed layer of each top-level module, from the architecture review
/// (`.local/archrev/report.html` section 5). `worktrees` (layer 4) was added after it:
/// it names `teardown`. `effort_journal` (layer 2) was added after it too: the effort
/// journal vocabulary the meta writer journals a reseat discard with, moved out of
/// `effort` (layer 4). The `#[cfg(test)]` fixture modules have no entry: no production
/// file names them. This table is the single source: the layer map in
/// `docs/reference/contract.md` mirrors it and `layers_map.rs` fails when the two differ.
pub(super) const MODULE_LAYERS: [(usize, &[&str]); 7] = [
    (
        0,
        &[
            "json",
            "time",
            "token",
            "event_text",
            "attention",
            "provenance",
            "error",
            "words",
            "sanitize",
            "redact",
        ],
    ),
    (
        1,
        &[
            "doors",
            "transport",
            "procs",
            "git",
            "shape",
            "instance",
            "tmux_floor",
            "netprobe",
        ],
    ),
    (
        2,
        &[
            "config",
            "meta",
            "tool",
            "harness_state",
            "launch_cmd",
            "theme",
            "store",
            "events",
            "effort_journal",
            "state",
            "requests",
            "inventory",
            "tracked",
            "roster",
            "pairs",
            "tmux",
            "session_tmux",
            "archive",
            "digest",
            "filters",
            "memo",
            "goal",
            "wait_on",
            "birth",
            "model_drift",
            "liveness",
            "conversation",
            "watchdog",
        ],
    ),
    (
        3,
        &[
            "deliver",
            "send",
            "reply",
            "relay",
            "brief",
            "brief_retry",
            "board",
            "quota",
            "usage",
            "go_usage",
            "muse_usage",
            "stats",
            "stats_store",
            "tool_stats",
            "telegram",
            "render",
            "listing",
            "fleet_read",
            "session",
            "launch",
            "run",
            "install",
            "upgrade",
            "autoupgrade",
            "init",
            "compact",
            "carry",
            "opencode_variant",
        ],
    ),
    (
        4,
        &[
            "session_launch",
            "request_verb",
            "effort",
            "migrate",
            "lifecycle",
            "spawn",
            "reseat",
            "seat_relaunch",
            "rename",
            "autoreseat",
            "autotrust",
            "watchdog_daemon",
            "watchdog_glue",
            "watchdog_lifecycle",
            "telegram_lifecycle",
            "orchestrator",
            "seatpack",
            "seatcompact",
            "seatcompact_run",
            "restore",
            "teardown",
            "interrupt",
            "session_notice",
            "worktrees",
        ],
    ),
    (
        5,
        &[
            "console",
            "clipboard",
            "doctor",
            "app",
            "session_menu",
            "settings_menu",
            "overview",
            "profile_view",
            "monitor",
            "jump",
            "panes",
            "next",
            "events_tail",
            "reader",
        ],
    ),
    (6, &["cli", "entry", "identity", "shim"]),
];

/// The reviewed layer of a top-level module, if it has one.
fn module_layer(module: &str) -> Option<usize> {
    MODULE_LAYERS
        .iter()
        .find(|(_, modules)| modules.contains(&module))
        .map(|(layer, _)| *layer)
}

/// The layer modules, bottom first: `src/l0_base.rs` beside `src/l0_base/` holds layer 0. A
/// layer module is the folder of its layer, never a module of the table.
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

/// The layer folder a `src/` path lies in or is the file of: `src/l0_base.rs` and
/// `src/l0_base/<module>.rs` are both in layer 0; a flat `src/<module>.rs` is in none.
fn folder_layer(file: &str) -> Option<usize> {
    let first = file.strip_prefix("src/")?.split('/').next()?;
    layer_dir(first.strip_suffix(".rs").unwrap_or(first))
}

/// Whether `file` is a layer file itself: `src/l0_base.rs`.
fn is_layer_file(file: &str) -> bool {
    file.strip_prefix("src/")
        .and_then(|rest| rest.strip_suffix(".rs"))
        .is_some_and(|name| layer_dir(name).is_some())
}

/// The top-level module of a `src/` file as the table names it, under the folder grammar:
/// `src/<module>/x.rs`, `src/l<N>_<word>/<module>/x.rs` and a flat `src/<module>.rs` are all
/// `<module>`.
/// `None` for `src/lib.rs` and `src/main.rs`, which are the top layer themselves, and for a
/// layer file, which is the folder of its layer.
fn top_module(file: &str) -> Result<Option<&str>, String> {
    let rest = file
        .strip_prefix("src/")
        .ok_or_else(|| format!("{file}: not a src/ file, so it has no layer"))?;
    if rest == "lib.rs" || rest == "main.rs" {
        return Ok(None);
    }
    let mut parts = rest.split('/');
    let first = parts.next().unwrap_or(rest);
    let first = first.strip_suffix(".rs").unwrap_or(first);
    if layer_dir(first).is_some() {
        return Ok(parts
            .next()
            .map(|module| module.strip_suffix(".rs").unwrap_or(module)));
    }
    Ok(Some(first))
}

/// The layer a file belongs to, or the reason it has none: the layer folder it lies in,
/// else the reviewed layer of its top-level module.
fn file_layer(file: &str) -> Result<usize, String> {
    if let Some(layer) = folder_layer(file) {
        return Ok(layer);
    }
    match top_module(file)? {
        None => Ok(TOP),
        Some(module) => module_layer(module).ok_or_else(|| unreviewed(file, module)),
    }
}

fn unreviewed(file: &str, module: &str) -> String {
    format!(
        "{file}: module `{module}` has no reviewed layer; add it to MODULE_LAYERS in \
         tests/it/doors/layers.rs (a new module is a layer review, never a default)"
    )
}

/// The layer a `crate::<first>` path names: a layer module's own layer, the module's, or
/// the top layer when `first` is no module, which makes it an item of `src/lib.rs`. A raw
/// identifier (`r#time`) names the module its plain spelling names; only the pin's key
/// keeps the spelling as written.
fn target_layer(first: &str) -> usize {
    let plain = first.strip_prefix("r#").unwrap_or(first);
    layer_dir(plain)
        .or_else(|| module_layer(plain))
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

/// The findings of one file's PRODUCTION text against `allow`, from the layer of its path.
/// Rows of other files are not this file's business: they are neither a grant nor slack here.
fn check(file: &str, production: &str, allow: &[Row]) -> Result<Vec<String>, String> {
    check_from(file, file_layer(file)?, production, allow)
}

/// [`check`] from layer `from`: in a whole tree, the owner the D4 walk gave the file. A
/// `crate::l<N>_…` spelling names a layer module directly, which only src/lib.rs, the layer
/// files and a `pub(in …)` scope may do; everyone else names the module by its root alias.
fn check_from(
    file: &str,
    from: usize,
    production: &str,
    allow: &[Row],
) -> Result<Vec<String>, String> {
    let canonical = file == "src/lib.rs" || is_layer_file(file);
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
        let to = target_layer(&spelling.first);
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
    file_layer(file)?;
    let class = source_view::classify(file, source).map_err(|failure| failure.to_string())?;
    check(file, &class.production, allow)
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

/// The findings of a whole `src/` tree, `files` as `(path, raw text)`, against `allow`:
/// the declarations of src/lib.rs and the layer files, the D4 owner walk, and each production
/// file's spellings from its owner. `Err` is a tree the guard cannot place: a file the shared
/// view cannot read, classify or reach, or a declaration file syn cannot parse. A policy
/// refusal is a finding that names its `file:line`. The real-tree guard and the frozen
/// fixtures both enter here.
pub(super) fn violations_in(
    files: &[(String, String)],
    allow: &[Row],
) -> Result<Vec<String>, String> {
    let product = source_view::product_of(files).map_err(|failure| failure.to_string())?;
    let mut classified = BTreeMap::new();
    for (path, text) in files {
        let class = source_view::classify(path, text).map_err(|failure| failure.to_string())?;
        classified.insert(path.as_str(), class);
    }
    let mut findings = declarations(files)?;
    let walk = owner_walk(files, &classified)?;
    findings.extend(walk.findings);
    for file in product
        .files
        .iter()
        .filter(|file| file.path.starts_with("src/"))
    {
        let from = if ROOT_FILES.contains(&file.path.as_str()) {
            TOP
        } else if let Some((owner, _)) = walk.owners.get(&file.path) {
            owner.layer
        } else {
            // A file the walk could not own already has its finding.
            continue;
        };
        findings.extend(check_from(&file.path, from, &file.text, allow)?);
    }
    Ok(findings)
}

// ---- the owner walk (D4) -----------------------------------------------------

/// The crate roots: the top layer, and where every registration chain starts.
const ROOT_FILES: [&str; 2] = ["src/lib.rs", "src/main.rs"];

/// Who owns a production file: its layer, and whether a layer file (`src/l<N>_<word>.rs`)
/// or a flat top-level module of `MODULE_LAYERS` started its registration chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Owner {
    layer: usize,
    foldered: bool,
}

impl Owner {
    fn describe(self) -> String {
        if self.foldered {
            format!(
                "layer {} ({}) through src/{}.rs",
                self.layer, LAYER_NAMES[self.layer], LAYER_DIRS[self.layer]
            )
        } else {
            format!(
                "layer {} ({}) through a flat module",
                self.layer, LAYER_NAMES[self.layer]
            )
        }
    }

    /// Where a file this owner reaches must lie, or `None` when `file` lies there.
    fn misplaced(self, file: &str, from_root: bool) -> Option<String> {
        let dir = LAYER_DIRS[self.layer];
        if self.foldered {
            let at_home = if from_root {
                file == format!("src/{dir}.rs")
            } else {
                file.starts_with(&format!("src/{dir}/"))
            };
            (!at_home).then(|| format!("outside src/{dir}/, the folder of its owner"))
        } else {
            folder_layer(file).map(|layer| {
                format!(
                    "inside the layer folder of {}, but its owner is a flat module",
                    LAYER_DIRS[layer]
                )
            })
        }
    }
}

/// The walk's result: every production file it owned, with its owner and the registration
/// that first reached it, the files placed where their owner forbids, and the findings.
struct Walk {
    owners: BTreeMap<String, (Owner, String)>,
    misplaced: BTreeSet<String>,
    findings: Vec<String>,
}

/// One ungated registration the walk follows: the file it names, the owner it carries, its
/// `file:line`, the module name, and whether a crate root wrote it.
struct Edge {
    target: String,
    owner: Owner,
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

/// The ungated registrations of the crate roots, each with the owner it starts: a layer
/// module that src/lib.rs registers starts its layer, and until P7 a flat top-level module
/// starts the layer `MODULE_LAYERS` gives it. A flat module without a row is a finding.
fn root_edges(
    classified: &BTreeMap<&str, source_view::Classified>,
    known: &[&str],
    findings: &mut Vec<String>,
) -> Result<Vec<Edge>, String> {
    let mut edges = Vec::new();
    for root in ROOT_FILES {
        let Some(class) = classified.get(root) else {
            continue;
        };
        for reg in class.registrations.iter().filter(|reg| !reg.gated) {
            let site = format!("{root}:{}", reg.line);
            let layer = (root == ROOT_FILES[0] && reg.nesting.is_empty())
                .then(|| layer_dir(&reg.name))
                .flatten();
            let owner = match (layer, module_layer(&reg.name)) {
                (Some(layer), _) => Owner {
                    layer,
                    foldered: true,
                },
                (None, Some(layer)) => Owner {
                    layer,
                    foldered: false,
                },
                (None, None) => {
                    findings.push(unreviewed(&site, &reg.name));
                    continue;
                }
            };
            edges.push(Edge {
                target: src_target(root, reg, known)?,
                owner,
                site,
                name: reg.name.clone(),
                from_root: true,
            });
        }
    }
    Ok(edges)
}

/// D4: carry an owner from the roots through every ungated registration. Every reached file
/// must lie where its owner says, no two owners may reach one file, and the module a file
/// belongs to must have the owner's layer in `MODULE_LAYERS`. Gated registrations and
/// everything below them are test-only and exempt.
fn owner_walk(
    files: &[(String, String)],
    classified: &BTreeMap<&str, source_view::Classified>,
) -> Result<Walk, String> {
    let known: Vec<&str> = files.iter().map(|(path, _)| path.as_str()).collect();
    let mut walk = Walk {
        owners: BTreeMap::new(),
        misplaced: BTreeSet::new(),
        findings: Vec::new(),
    };
    let mut edges = root_edges(classified, &known, &mut walk.findings)?;
    let mut queue = std::collections::VecDeque::new();
    loop {
        for edge in edges.drain(..) {
            let Edge {
                target,
                owner,
                site,
                name,
                from_root,
            } = edge;
            if let Some(place) = owner.misplaced(&target, from_root) {
                walk.findings.push(format!(
                    "{site}: `mod {name}` places {target} {place}, {}",
                    owner.describe()
                ));
                walk.misplaced.insert(target.clone());
            }
            match walk.owners.get(&target) {
                None => {
                    walk.owners.insert(target.clone(), (owner, site));
                    queue.push_back(target);
                }
                Some((first, first_site)) if *first != owner => {
                    walk.findings.push(format!(
                        "{target}: two owners reach it, {first_site} ({}) and {site} ({}); a \
                         production file has one owner",
                        first.describe(),
                        owner.describe()
                    ));
                }
                Some(_) => {}
            }
        }
        let Some(file) = queue.pop_front() else {
            break;
        };
        let (Some(owner), Some(class)) = (
            walk.owners.get(&file).map(|(owner, _)| *owner),
            classified.get(file.as_str()),
        ) else {
            continue;
        };
        for reg in class.registrations.iter().filter(|reg| !reg.gated) {
            let site = format!("{file}:{}", reg.line);
            // A layer file introduces top-level modules: each one's own name must carry the
            // owner's layer, whatever file its `#[path]` loads.
            if is_layer_file(&file) && reg.nesting.is_empty() {
                match module_layer(&reg.name) {
                    None => walk.findings.push(unreviewed(&site, &reg.name)),
                    Some(layer) if layer != owner.layer => walk.findings.push(format!(
                        "{site}: `mod {}` registers a module that MODULE_LAYERS gives layer \
                         {layer} ({}), but {}",
                        reg.name,
                        LAYER_NAMES[layer],
                        owner.describe()
                    )),
                    Some(_) => {}
                }
            }
            edges.push(Edge {
                target: src_target(&file, reg, &known)?,
                owner,
                site,
                name: reg.name.clone(),
                from_root: false,
            });
        }
    }
    walk.findings.extend(table_findings(&walk)?);
    Ok(walk)
}

/// The module each placed file belongs to must have its owner's layer in `MODULE_LAYERS`.
fn table_findings(walk: &Walk) -> Result<Vec<String>, String> {
    let mut findings = Vec::new();
    for (file, (owner, site)) in &walk.owners {
        if walk.misplaced.contains(file) {
            continue;
        }
        let Some(module) = top_module(file)? else {
            continue;
        };
        match module_layer(module) {
            None => findings.push(format!(
                "{site}: {file} is module `{module}`, which has no reviewed layer; add it to \
                 MODULE_LAYERS in tests/it/doors/layers.rs (a new module is a layer review, \
                 never a default)"
            )),
            Some(layer) if layer != owner.layer => findings.push(format!(
                "{site}: {file} is module `{module}` of {} through this registration, but \
                 MODULE_LAYERS gives `{module}` layer {layer} ({})",
                owner.describe(),
                LAYER_NAMES[layer]
            )),
            Some(_) => {}
        }
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
                module.ident.to_string(),
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

/// D1 and the D4 alias rule, on src/lib.rs and the layer files it registers: the flat block
/// declares only `pub mod` and `mod`, a layer file only `pub mod` and `pub(crate) mod`, no
/// flat module belongs to a foldered layer, and every root `use` through a layer module is
/// `l<N>_<word>::<module>`, binds the name `<module>` and names a module that its layer file
/// registers. A renamed alias or a re-export below or of the layer module would give a
/// higher layer a name that the spelling ratchet reads as another module's layer.
fn declarations(files: &[(String, String)]) -> Result<Vec<String>, String> {
    let text_of = |file: &str| {
        files
            .iter()
            .find(|(path, _)| path == file)
            .map(|(_, text)| text.as_str())
    };
    let Some(lib) = text_of("src/lib.rs") else {
        return Ok(Vec::new());
    };
    let mut findings = Vec::new();
    let flat = mod_declarations("src/lib.rs", lib)?;
    let foldered: BTreeSet<usize> = flat
        .iter()
        .filter_map(|(_, name, _)| layer_dir(name))
        .collect();
    let mut registered: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    for (vis, name, line) in &flat {
        if !vis.is_empty() && vis != "pub" {
            findings.push(format!(
                "src/lib.rs:{line}: `{vis} mod {name};` in the flat block, which declares a \
                 module only as `pub mod` or `mod` (D1)"
            ));
        }
        if let Some(layer) = module_layer(name).filter(|layer| foldered.contains(layer)) {
            findings.push(format!(
                "src/lib.rs:{line}: flat module `{name}` belongs to layer {layer} ({}), which is \
                 foldered: declare it in src/{}.rs",
                LAYER_NAMES[layer], LAYER_DIRS[layer]
            ));
        }
    }
    for layer in &foldered {
        let file = format!("src/{}.rs", LAYER_DIRS[*layer]);
        let Some(text) = text_of(&file) else {
            continue;
        };
        let names = registered.entry(*layer).or_default();
        for (vis, name, line) in mod_declarations(&file, text)? {
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
            names.insert(name);
        }
    }
    findings.extend(alias_findings(&root_uses(lib)?, &registered));
    Ok(findings)
}

/// The D1 rule on root `use`s through a layer module: each is `l<N>_<word>::<module>`, binds
/// `<module>` and names a module that `registered` lists for its layer.
fn alias_findings(
    leaves: &[UseLeaf],
    registered: &BTreeMap<usize, BTreeSet<String>>,
) -> Vec<String> {
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
                let known = registered
                    .get(&layer)
                    .is_some_and(|names| names.contains(module));
                if !known {
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
fn every_production_module_has_one_reviewed_layer_and_every_entry_is_live() {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for (layer, modules) in MODULE_LAYERS {
        for module in modules {
            *seen.entry(*module).or_default() += 1;
            assert!(
                layer < LAYER_NAMES.len(),
                "{module}: layer {layer} does not exist"
            );
        }
    }
    let twice: Vec<_> = seen
        .iter()
        .filter(|(_, n)| **n > 1)
        .map(|(m, _)| *m)
        .collect();
    assert!(twice.is_empty(), "modules listed in two layers: {twice:?}");

    let files = product_files();
    let produced: BTreeSet<&str> = files
        .iter()
        .filter_map(|(path, _)| top_module(path).ok().flatten())
        .collect();
    let listed: BTreeSet<&str> = seen.keys().copied().collect();
    let unlisted: Vec<_> = produced.difference(&listed).collect();
    let stale: Vec<_> = listed.difference(&produced).collect();
    assert!(
        unlisted.is_empty(),
        "production modules with no reviewed layer: {unlisted:?}"
    );
    assert!(
        stale.is_empty(),
        "layer table entries with no production module: {stale:?}"
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

#[test]
fn p0_review_a_layer_file_registration_takes_the_layer_of_its_own_name() {
    // Review r1 (layout-nav): a placed, table-listed file registered under another module's
    // name. The alias `crate::time` would read as layer 0 while the file is owned by layer 5.
    let renamed = fixture_tree(&[
        ("lib", "mod l5_ui;\npub use l5_ui::time;\n"),
        ("l5_ui", "#[path = \"l5_ui/console.rs\"]\npub mod time;\n"),
        ("l5_ui/console", "pub fn shown() {}\n"),
    ]);
    let found = violations_in(&renamed, &[]).unwrap_or_else(|why| panic!("{why}"));
    assert!(
        found
            .iter()
            .any(|finding| finding.contains("`mod time`") && finding.contains("layer 0")),
        "{found:?}"
    );
    // Control: the same file registered under its own name is accepted.
    let own = fixture_tree(&[
        ("lib", "mod l5_ui;\npub use l5_ui::console;\n"),
        ("l5_ui", "pub mod console;\n"),
        ("l5_ui/console", "pub fn shown() {}\n"),
    ]);
    assert_eq!(violations_in(&own, &[]), Ok(Vec::new()));
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
