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

use super::layers_allow::{DEBT, HANDSHAKE, Row};
use crate::support::source_view;

/// The layer names, bottom first. A file may name its own layer and the ones before it.
const LAYER_NAMES: [&str; 7] = [
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
/// file names them.
const MODULE_LAYERS: [(usize, &[&str]); 7] = [
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

/// The top-level module of a `src/` file as the table names it: `src/app/draw/x.rs` is
/// `app`, `src/json.rs` is `json`. `None` for `src/lib.rs` and `src/main.rs`, which are
/// the top layer themselves.
fn top_module(file: &str) -> Result<Option<&str>, String> {
    let rest = file
        .strip_prefix("src/")
        .ok_or_else(|| format!("{file}: not a src/ file, so it has no layer"))?;
    if rest == "lib.rs" || rest == "main.rs" {
        return Ok(None);
    }
    let first = rest.split('/').next().unwrap_or(rest);
    Ok(Some(first.strip_suffix(".rs").unwrap_or(first)))
}

/// The layer a file belongs to, or the reason it has none.
fn file_layer(file: &str) -> Result<usize, String> {
    match top_module(file)? {
        None => Ok(TOP),
        Some(module) => module_layer(module).ok_or_else(|| {
            format!(
                "{file}: module `{module}` has no reviewed layer; add it to MODULE_LAYERS in \
                 tests/it/doors/layers.rs (a new module is a layer review, never a default)"
            )
        }),
    }
}

/// The layer a `crate::<first>` path names: the module's, or the top layer when `first`
/// is no module, which makes it an item of `src/lib.rs`. A raw identifier (`r#time`) names
/// the module its plain spelling names; only the pin's key keeps the spelling as written.
fn target_layer(first: &str) -> usize {
    module_layer(first.strip_prefix("r#").unwrap_or(first)).unwrap_or(TOP)
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
                walk(group.stream(), out);
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

/// The findings of one file's PRODUCTION text against `allow`. Rows of other files are
/// not this file's business: they are neither a grant nor slack here.
fn check(file: &str, production: &str, allow: &[Row]) -> Result<Vec<String>, String> {
    let from = file_layer(file)?;
    let mut upward: BTreeMap<String, Upward> = BTreeMap::new();
    for spelling in spellings(file, production)? {
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

    let mut findings = Vec::new();
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

// ---- the real tree -----------------------------------------------------------

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
    let rows = real_rows();
    let mut findings = Vec::new();
    for (path, production) in product_files() {
        match check(path, production, &rows) {
            Ok(found) => findings.extend(found),
            Err(why) => findings.push(why),
        }
    }
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

#[test]
fn a_raw_identifier_names_the_same_module_as_its_plain_spelling() {
    let found = |file: &str, source: &str, allow: &[Row]| {
        violations_with(file, source, allow).unwrap_or_else(|why| panic!("{why}"))
    };
    // `r#time` is the module `time`: sideways from a layer 0 file and downward from the CLI
    // shell, so neither is a finding.
    assert!(found("src/json.rs", "use crate::r#time;", &[]).is_empty());
    assert!(found("src/cli.rs", "use crate::r#time::Stamp;", &[]).is_empty());
    // `r#cli` is the module `cli`: upward from layer 0, and the pin keys on the spelling as
    // written, so the plain spelling's row does not admit it and its own row does.
    let up = found("src/json.rs", "use crate::r#cli::Request;", &[]);
    assert_eq!(up.len(), 1, "{up:?}");
    assert!(up[0].contains("`crate::r#cli::Request`"), "{up:?}");
    let plain = [("src/json.rs", "crate::cli::Request", 1)];
    assert!(!found("src/json.rs", "use crate::r#cli::Request;", &plain).is_empty());
    let raw = [("src/json.rs", "crate::r#cli::Request", 1)];
    assert!(found("src/json.rs", "use crate::r#cli::Request;", &raw).is_empty());
    // A raw root item is still a root item: layer 6.
    assert!(!found("src/json.rs", "fn live() { crate::r#type(); }", &[]).is_empty());
}
