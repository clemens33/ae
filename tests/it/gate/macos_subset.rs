//! The Mac boundary subset and its guard (plan r2 §4, lead ruling C).
//!
//! A release runs the WHOLE suite on the hp and only this subset on the Mac:
//! every unit test, plus the integration modules that touch a Mac boundary, plus
//! one pinned test per shared boundary helper, plus the Mac-only regressions.
//! The subset lives in `.config/macos-subset`; `just test-macos` renders it into a
//! nextest filterset. This guard derives the boundary from the source and refuses
//! a manifest that misses part of it:
//!
//! * a MARKER SITE (an OS conditional, the process table, a real tmux client, a
//!   justfile recipe, fsync, the clipboard, an archive tool) in an integration
//!   module no `module` or `history` entry covers;
//! * a member of the nextest `tmux-clients` test group no entry covers, or a
//!   `tmux-clients` line this guard cannot read;
//! * a SHARED BOUNDARY HELPER (a non-test fn in tests/ that reaches
//!   `Command::new`, `env!("CARGO_BIN_EXE_ae")` or `.spawn()`, called from another
//!   file) without its pinned member, a member this guard cannot prove drives it,
//!   or a pinned helper that no longer is one;
//! * an entry that resolves to no module or test.
//!
//! A pinned member counts only when this guard PROVES it calls the helper: through a
//! path, `self`, or a receiver whose type it resolves from a declaration (a typed
//! parameter or `let`, a declared return or field type, a unit-struct value or a struct
//! literal). A receiver it cannot type only widens the helper set; it never certifies a
//! member, so an unsupported form refuses and the fix is a simpler member.
//!
//! The threat model is an honest mistake by our own agents, not contrived Rust; the
//! release's whole hp run is the backstop. Known limits, which neither certify nor refuse
//! on their own:
//!
//! * a method resolves on the receiver's own declared type: a `Deref` to another type, a
//!   trait method implemented for a std type (`Option<T>`, `Vec<T>`), a method defined by
//!   a macro, or an inherent impl in another module than its type is not seen;
//! * `clone`, `unwrap` and `expect` without a declared method of that name are taken as
//!   the std methods; `Option` and `Result` are the only wrappers counted, and `Box`,
//!   `Arc` and `Rc` are seen through;
//! * names bound by macros, type aliases and generic parameters are not typed;
//! * a `CARGO_BIN_EXE_ae` primitive is the `env!` / `option_env!` / `var` / `var_os` read.
//!
//! Pure: the caller hands in the files (the frozen spec's `real_files`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::modtree::{self, Crate, Files, Kind, Lexed, ModPath, TreeError, join_path, read_uses};

/// The nextest config whose `tmux-clients` group members are boundary modules.
pub(crate) const NEXTEST: &str = ".config/nextest.toml";
const CLIENT_GROUP: &str = "tmux-clients";

/// Why a site is a Mac boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Class {
    OsConditional,
    ProcessTable,
    TmuxClient,
    Recipe,
    Fsync,
    Clipboard,
    ArchiveTool,
}

/// One boundary marker, attributed to one owning module (a file with two owners has two sites).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Site {
    pub(crate) class: Class,
    pub(crate) file: String,
    pub(crate) line: usize,
    pub(crate) module: ModPath,
}

/// A shared boundary helper: `item` is `Type::fn` or `fn`; `callers` are the full
/// names of the tests that drive it in a normal run.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Helper {
    pub(crate) file: String,
    pub(crate) item: String,
    pub(crate) line: usize,
    pub(crate) callers: Vec<String>,
}

/// One manifest line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    Unit,
    Module(String),
    Test(String),
    Helper { item: String, member: String },
    History { target: String, issue: String },
}

/// One reason the manifest does not cover the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Finding {
    Malformed { line: usize, text: String },
    UnitMissingOrRepeated,
    UncoveredSite(Site),
    HelperWithoutMember(Helper),
    StaleHelper(String),
    MemberNotCaller { item: String, member: String },
    UnknownModule(String),
    UnresolvedTree(TreeError),
    GroupFilter { line: usize, text: String },
}

// ---------------------------------------------------------------------------
// The manifest and its filterset
// ---------------------------------------------------------------------------

/// Parse the manifest. `#` starts a comment, except that a `history` line's
/// comment is its issue number. Exactly one `unit` line. A line holds printable
/// ASCII and tabs only, as `just test-macos` reads it: a CR or any other byte refuses.
pub(crate) fn parse_manifest(text: &str) -> Result<Vec<Entry>, Vec<Finding>> {
    let mut entries = Vec::new();
    let mut findings = Vec::new();
    for (at, raw) in text.split('\n').enumerate() {
        match parse_line(raw) {
            Ok(Some(entry)) => entries.push(entry),
            Ok(None) => {}
            Err(()) => findings.push(Finding::Malformed {
                line: at + 1,
                text: raw.to_owned(),
            }),
        }
    }
    if entries
        .iter()
        .filter(|entry| **entry == Entry::Unit)
        .count()
        != 1
    {
        findings.push(Finding::UnitMissingOrRepeated);
    }
    if findings.is_empty() {
        Ok(entries)
    } else {
        Err(findings)
    }
}

fn parse_line(raw: &str) -> Result<Option<Entry>, ()> {
    if !raw
        .bytes()
        .all(|byte| byte == b'\t' || (b' '..=b'~').contains(&byte))
    {
        return Err(());
    }
    let (body, comment) = raw
        .split_once('#')
        .map_or((raw, None), |(body, rest)| (body, Some(rest)));
    let words: Vec<&str> = body.split_whitespace().collect();
    let entry = match words.as_slice() {
        [] => return Ok(None),
        ["unit"] => Entry::Unit,
        ["module", module] if is_path(module) => Entry::Module((*module).to_owned()),
        ["test", test] if is_path(test) => Entry::Test((*test).to_owned()),
        ["helper", item, "=", member] if is_item(item) && is_path(member) => Entry::Helper {
            item: (*item).to_owned(),
            member: (*member).to_owned(),
        },
        ["history", target] if is_path(target) => {
            let issue = comment.map(str::trim).unwrap_or_default();
            if issue.is_empty() || !issue.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(());
            }
            Entry::History {
                target: (*target).to_owned(),
                issue: issue.to_owned(),
            }
        }
        _ => return Err(()),
    };
    Ok(Some(entry))
}

/// `a::b_c::d`: identifier segments joined by `::`.
fn is_path(text: &str) -> bool {
    text.split("::").all(|segment| {
        segment
            .bytes()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    })
}

/// `tests/it/x.rs::Type::fn`: a repository path ending in `.rs`, then a path.
fn is_item(text: &str) -> bool {
    text.split_once(".rs::").is_some_and(|(file, path)| {
        !file.is_empty()
            && file.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'/' | b'-')
            })
            && is_path(path)
    })
}

/// The nextest filterset of the subset; `just test-macos` prints the same text.
pub(crate) fn filterset(entries: &[Entry]) -> String {
    fn push<'a>(list: &mut Vec<&'a str>, name: &'a str) {
        if !list.contains(&name) {
            list.push(name);
        }
    }
    let mut modules: Vec<&str> = Vec::new();
    let mut exact: Vec<&str> = Vec::new();
    for entry in entries {
        match entry {
            Entry::Unit => {}
            Entry::Module(module) => push(&mut modules, module),
            Entry::Test(test) | Entry::Helper { member: test, .. } => push(&mut exact, test),
            Entry::History { target, .. } => {
                push(&mut modules, target);
                push(&mut exact, target);
            }
        }
    }
    let mut terms: Vec<String> = Vec::new();
    if !modules.is_empty() {
        terms.push(format!("test(/^({})::/)", modules.join("|")));
    }
    terms.extend(exact.iter().map(|test| format!("test(={test})")));
    if terms.is_empty() {
        "kind(lib) | kind(bin)".to_owned()
    } else {
        format!(
            "kind(lib) | kind(bin) | (binary_id(ae::it) & ({}))",
            terms.join(" | ")
        )
    }
}

// ---------------------------------------------------------------------------
// Marker sites
// ---------------------------------------------------------------------------

/// Every marker site of every file in `modules`, plus the readable `tmux-clients` members.
pub(crate) fn sites(files: &Files, modules: &BTreeMap<String, Vec<ModPath>>) -> Vec<Site> {
    let mut found = BTreeSet::new();
    for (file, owners) in modules {
        let Some(text) = files.get(file) else {
            continue;
        };
        let lexed = modtree::lex(text);
        let spans = lexed.inline_modules();
        for (class, index) in markers(&lexed) {
            let inline = Lexed::inline_path(&spans, index);
            let line = lexed.line(lexed.tokens[index].start);
            for owner in owners {
                found.insert(Site {
                    class,
                    file: file.clone(),
                    line,
                    module: ModPath {
                        krate: owner.krate,
                        path: join_path(&owner.path, &inline),
                    },
                });
            }
        }
    }
    found.extend(client_group(files).0);
    found.into_iter().collect()
}

fn markers(lexed: &Lexed) -> Vec<(Class, usize)> {
    let mut found = Vec::new();
    for index in 0..lexed.tokens.len() {
        if let Some(name) = lexed.ident(index) {
            let after = |segment: &str| {
                index >= 3 && lexed.is_path_sep(index - 2) && lexed.is_ident(index - 3, segment)
            };
            let class = match name {
                "target_os" | "uname" => Some(Class::OsConditional),
                "OS" if after("consts") => Some(Class::OsConditional),
                "procs" if after("ae") => Some(Class::ProcessTable),
                "sysctl" | "boottime" => Some(Class::ProcessTable),
                "sync_all" | "sync_data" | "fsync" | "F_FULLFSYNC" => Some(Class::Fsync),
                _ => None,
            };
            found.extend(class.map(|class| (class, index)));
        } else if let Some(value) = lexed.literal(index) {
            found.extend(
                literal_classes(value)
                    .into_iter()
                    .map(|class| (class, index)),
            );
        }
    }
    found
}

fn literal_classes(value: &str) -> Vec<Class> {
    let has = |needle: &str| value.contains(needle);
    let rules = [
        (
            Class::OsConditional,
            has("Darwin") || has_word(value, "uname"),
        ),
        (
            Class::ProcessTable,
            value == "ps" || has("sysctl") || has("kern.") || has("boottime"),
        ),
        (Class::TmuxClient, has("attach-session")),
        (Class::Recipe, value == "just" || has("justfile")),
        (Class::Fsync, has("F_FULLFSYNC")),
        (
            Class::Clipboard,
            has("pbcopy") || has("pbpaste") || has("osascript"),
        ),
        (
            Class::ArchiveTool,
            value == "tar" || has("codesign") || has("xattr"),
        ),
    ];
    rules
        .into_iter()
        .filter(|(_, hit)| *hit)
        .map(|(class, _)| class)
        .collect()
}

fn has_word(value: &str, word: &str) -> bool {
    let ident =
        |byte: Option<u8>| byte.is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    value.match_indices(word).any(|(at, _)| {
        !ident(
            value
                .as_bytes()
                .get(at.wrapping_sub(1))
                .copied()
                .filter(|_| at > 0),
        ) && !ident(value.as_bytes().get(at + word.len()).copied())
    })
}

/// The `tmux-clients` members as sites, and every filter of that group this guard cannot read.
fn client_group(files: &Files) -> (Vec<Site>, Vec<Finding>) {
    let mut sites = Vec::new();
    let mut findings = Vec::new();
    let Some(text) = files.get(NEXTEST) else {
        return (sites, findings);
    };
    let mut table: Vec<(usize, String, String)> = Vec::new();
    let mut flush = |table: &mut Vec<(usize, String, String)>| {
        let group = table
            .iter()
            .find(|(_, key, _)| key == "test-group")
            .map(|(_, _, value)| value.clone());
        if group.as_deref() == Some(CLIENT_GROUP) {
            match table.iter().find(|(_, key, _)| key == "filter") {
                Some((line, _, filter)) => match client_modules(filter) {
                    Some(members) => sites.extend(members.into_iter().map(|module| Site {
                        class: Class::TmuxClient,
                        file: NEXTEST.to_owned(),
                        line: *line,
                        module: ModPath {
                            krate: Crate::It,
                            path: module,
                        },
                    })),
                    None => findings.push(Finding::GroupFilter {
                        line: *line,
                        text: filter.clone(),
                    }),
                },
                None => findings.push(Finding::GroupFilter {
                    line: table.first().map_or(0, |(line, _, _)| *line),
                    text: "a tmux-clients override without a filter".to_owned(),
                }),
            }
        }
        table.clear();
    };
    // A line this guard cannot read must not hide a member: a unicode escape or a
    // multi-line string anywhere, or `tmux-clients` outside the group header and a plain
    // `test-group` assignment, refuses.
    let mut unread = Vec::new();
    for (at, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.starts_with('#') {
            continue;
        }
        let unreadable = |text: &str| Finding::GroupFilter {
            line: at + 1,
            text: text.to_owned(),
        };
        if ["\\u", "\\U", "'''", "\"\"\""]
            .iter()
            .any(|form| line.contains(form))
        {
            unread.push(unreadable(line));
            continue;
        }
        if line.starts_with('[') {
            flush(&mut table);
            let header: Vec<&str> = line
                .trim_matches(|byte| byte == '[' || byte == ']')
                .split('.')
                .map(|segment| unquote(segment.trim()))
                .collect();
            if line.contains(CLIENT_GROUP) && header != ["test-groups", CLIENT_GROUP] {
                unread.push(unreadable(line));
            }
        } else if let Some((key, value)) = line.split_once('=') {
            let key = unquote(key.trim());
            let value = toml_string(value.trim());
            if line.contains(CLIENT_GROUP) && (key != "test-group" || value != CLIENT_GROUP) {
                unread.push(unreadable(line));
            }
            table.push((at + 1, key.to_owned(), value));
        } else if line.contains(CLIENT_GROUP) {
            unread.push(unreadable(line));
        }
    }
    flush(&mut table);
    findings.extend(unread);
    (sites, findings)
}

/// A TOML bare or quoted key's name.
fn unquote(key: &str) -> &str {
    for quote in ['\'', '"'] {
        if let Some(inner) = key
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    key
}

/// A TOML basic or literal string's contents; anything else as written.
fn toml_string(value: &str) -> String {
    for quote in ['\'', '"'] {
        if let Some(rest) = value.strip_prefix(quote)
            && let Some((inner, _)) = rest.split_once(quote)
        {
            return inner.to_owned();
        }
    }
    value.to_owned()
}

/// `test(/^(a|b::c)::/)` or `test(/^a::/)` to its module paths; any other form is unreadable.
fn client_modules(filter: &str) -> Option<Vec<String>> {
    let inner = filter.strip_prefix("test(/^")?.strip_suffix("::/)")?;
    let inner = inner
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or(inner);
    let members: Vec<String> = inner.split('|').map(str::to_owned).collect();
    members
        .iter()
        .all(|member| is_path(member))
        .then_some(members)
}

// ---------------------------------------------------------------------------
// Tests and helpers
// ---------------------------------------------------------------------------

/// Every `#[test]` fn of the integration crate, by its full nextest name, sorted. The walk lives
/// in `modtree`, which the merge-gate selector shares.
pub(crate) use modtree::test_names;

/// Every shared boundary helper of the integration crate, with the tests that drive it.
pub(crate) fn helpers(files: &Files, modules: &BTreeMap<String, Vec<ModPath>>) -> Vec<Helper> {
    Index::build(files, modules).helpers()
}

/// One fn of an integration-crate file.
struct Node {
    file: String,
    typ: String,
    name: String,
    line: usize,
    test: bool,
    child_mode: bool,
    returns_self: bool,
    /// The module paths the fn lives in: each integration owner of its file plus the inline path.
    contexts: Vec<String>,
    body: (usize, usize),
    /// The named parameters (`self` excluded) with their declared types.
    params: Vec<(String, Option<TypeRef>)>,
    /// The declared return type.
    ret: Option<TypeRef>,
}

/// A type as written: the path before its name and the name, with references,
/// lifetimes, `Box` / `Arc` / `Rc` and `Option` / `Result` peeled; `layers` counts the
/// `Option` / `Result` layers, which a method call does not see through.
#[derive(Clone, Debug)]
struct TypeRef {
    path: Vec<String>,
    name: String,
    layers: usize,
}

/// A value's type, and how many `Option` / `Result` layers still wrap it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Value {
    typ: Type,
    layers: usize,
}

/// A name `use` brings into a module.
#[derive(Clone)]
enum Import {
    Module(String),
    Item(String, String),
}

/// A type as its (module path, type name).
type Type = (String, String);

/// Types as (module path, type name) pairs.
type TypeSet = BTreeSet<Type>;

/// The calls of one fn. A PROVEN edge names its callee through a path, `self`, or a
/// receiver whose type this guard resolves; only proven edges make a test a driving
/// member. A MAY edge is a same-named method of a type the file names, for a receiver
/// this guard cannot type; it only widens which fns count as helpers.
#[derive(Default)]
struct Edges {
    proven: BTreeSet<usize>,
    may: BTreeSet<usize>,
}

/// One name a fn binds, in scope for the tokens after `from` and before `until`.
struct Binding {
    name: String,
    from: usize,
    until: usize,
    bound: Bound,
}

enum Bound {
    /// A declared type.
    Typed(TypeRef),
    /// `let name = <tokens start..end>;`
    Init(usize, usize),
    /// A pattern this guard does not type.
    Opaque,
}

/// One fn body read from one of its module contexts.
struct Scope<'s, 'a> {
    node: &'s Node,
    context: &'s str,
    lexed: &'s Lexed<'a>,
    bindings: &'s [Binding],
}

impl Scope<'_, '_> {
    /// The type `Self` names in this fn.
    fn own_type(&self) -> Option<Type> {
        (!self.node.typ.is_empty()).then(|| (self.context.to_owned(), self.node.typ.clone()))
    }
}

/// How deep a receiver chain or a `let` initializer is followed.
const CHAIN_DEPTH: usize = 12;

#[derive(Default)]
struct Index<'a> {
    lexed: BTreeMap<String, Lexed<'a>>,
    nodes: Vec<Node>,
    by_key: BTreeMap<(String, String, String), Vec<usize>>,
    methods: BTreeMap<String, Vec<usize>>,
    modules: BTreeSet<String>,
    types: TypeSet,
    fields: BTreeMap<Type, BTreeMap<String, TypeRef>>,
    imports: BTreeMap<String, BTreeMap<String, Import>>,
    globs: BTreeMap<String, BTreeSet<String>>,
    drops: BTreeMap<Type, usize>,
}

impl<'a> Index<'a> {
    fn build(files: &'a Files, modules: &BTreeMap<String, Vec<ModPath>>) -> Self {
        let mut index = Index::default();
        let mut owned: Vec<(String, Vec<String>)> = Vec::new();
        for (file, owners) in modules {
            let roots: Vec<String> = owners
                .iter()
                .filter(|owner| owner.krate == Crate::It)
                .map(|owner| owner.path.clone())
                .collect();
            if roots.is_empty() || !files.contains_key(file) {
                continue;
            }
            index.modules.extend(roots.iter().cloned());
            owned.push((file.clone(), roots));
        }
        for (file, roots) in &owned {
            let Some(text) = files.get(file) else {
                continue;
            };
            let lexed = modtree::lex(text);
            index.read_file(file, roots, &lexed);
            index.lexed.insert(file.clone(), lexed);
        }
        for (file, roots) in &owned {
            if let Some(lexed) = index.lexed.get(file) {
                let uses = read_uses(lexed, roots);
                for (context, alias, path, glob) in uses {
                    index.import(&context, &alias, &path, glob);
                }
            }
        }
        index
    }

    /// Record the inline modules, types, struct fields, impls and fns of one file.
    fn read_file(&mut self, file: &str, roots: &[String], lexed: &Lexed) {
        let spans = lexed.inline_modules();
        let context_at = |at: usize| -> Vec<String> {
            let inline = Lexed::inline_path(&spans, at);
            roots.iter().map(|root| join_path(root, &inline)).collect()
        };
        for (open, _, name) in &spans {
            for context in context_at(*open) {
                self.modules.insert(join_path(&context, name));
            }
        }
        let impls = impl_blocks(lexed);
        let mut attrs: Vec<(usize, usize)> = Vec::new();
        let mut at = 0;
        while at < lexed.tokens.len() {
            if let Some(end) = lexed.attribute_end(at) {
                attrs.push((at, end));
                at = end + 1;
                continue;
            }
            match lexed.ident(at) {
                Some("pub") => {
                    at = lexed.skip_visibility(at);
                    continue;
                }
                Some("async" | "const" | "unsafe" | "extern") if lexed.ident(at + 1).is_some() => {
                    at += 1;
                    continue;
                }
                Some(keyword @ ("struct" | "enum" | "union" | "trait")) => {
                    if let Some(name) = lexed.ident(at + 1) {
                        let fields = if keyword == "struct" {
                            struct_fields(lexed, at + 1)
                        } else {
                            BTreeMap::new()
                        };
                        for context in context_at(at) {
                            let typ = (context, name.to_owned());
                            if !fields.is_empty() {
                                self.fields.insert(typ.clone(), fields.clone());
                            }
                            self.types.insert(typ);
                        }
                    }
                }
                Some("fn") => {
                    if let Some(item) = read_fn(lexed, at, &attrs, &impls) {
                        let contexts = context_at(at);
                        for context in &contexts {
                            if !item.typ.is_empty() {
                                self.types.insert((context.clone(), item.typ.clone()));
                            }
                        }
                        self.add(file, lexed, contexts, item);
                    }
                }
                _ => {}
            }
            attrs.clear();
            at += 1;
        }
    }

    fn add(&mut self, file: &str, lexed: &Lexed, contexts: Vec<String>, item: FnItem) {
        let at = self.nodes.len();
        for context in &contexts {
            self.by_key
                .entry((context.clone(), item.typ.clone(), item.name.clone()))
                .or_default()
                .push(at);
        }
        if !item.typ.is_empty() {
            self.methods.entry(item.name.clone()).or_default().push(at);
            if item.name == "drop" {
                for context in &contexts {
                    self.drops.insert((context.clone(), item.typ.clone()), at);
                }
            }
        }
        self.nodes.push(Node {
            file: file.to_owned(),
            child_mode: item.test && child_mode(lexed, item.body),
            line: lexed.line(item.start),
            typ: item.typ,
            name: item.name,
            test: item.test,
            returns_self: item.returns_self,
            contexts,
            body: item.body,
            params: item.params,
            ret: item.ret,
        });
    }

    fn import(&mut self, context: &str, alias: &str, path: &[String], glob: bool) {
        if glob {
            if let Some(module) = self.resolve_module(context, path) {
                self.globs
                    .entry(context.to_owned())
                    .or_default()
                    .insert(module);
            }
            return;
        }
        let entry = if let Some(module) = self.resolve_module(context, path) {
            Import::Module(module)
        } else if let Some((last, parent)) = path.split_last()
            && let Some(module) = self.resolve_module(context, parent)
        {
            Import::Item(module, last.clone())
        } else {
            return;
        };
        self.imports
            .entry(context.to_owned())
            .or_default()
            .insert(alias.to_owned(), entry);
    }

    /// The module path `segments` names from `context`, or `None` (an external crate, an item).
    fn resolve_module(&self, context: &str, segments: &[String]) -> Option<String> {
        let (first, rest) = segments.split_first()?;
        let mut module = match first.as_str() {
            "crate" => String::new(),
            "self" => context.to_owned(),
            "super" => parent_module(context)?,
            name => {
                let child = join_path(context, name);
                if self.modules.contains(&child) {
                    child
                } else if let Some(Import::Module(module)) =
                    self.imports.get(context).and_then(|names| names.get(name))
                {
                    module.clone()
                } else {
                    return None;
                }
            }
        };
        for segment in rest {
            module = if segment == "super" {
                parent_module(&module)?
            } else {
                join_path(&module, segment)
            };
            if !self.modules.contains(&module) && !module.is_empty() {
                return None;
            }
        }
        Some(module)
    }

    /// The (module, type) a type name or path names from `context`.
    fn resolve_type(&self, context: &str, path: &[String], typ: &str) -> Option<Type> {
        if !path.is_empty() {
            return self
                .resolve_module(context, path)
                .map(|module| (module, typ.to_owned()));
        }
        if self.types.contains(&(context.to_owned(), typ.to_owned())) {
            return Some((context.to_owned(), typ.to_owned()));
        }
        if let Some(Import::Item(module, name)) =
            self.imports.get(context).and_then(|names| names.get(typ))
        {
            return Some((module.clone(), name.clone()));
        }
        self.globs.get(context).and_then(|globs| {
            globs
                .iter()
                .find(|module| self.types.contains(&((*module).clone(), typ.to_owned())))
                .map(|module| (module.clone(), typ.to_owned()))
        })
    }

    /// The type a written type names from `context`; `Self` is `own`.
    fn resolve_ref(&self, context: &str, own: Option<&Type>, typ: &TypeRef) -> Option<Type> {
        if typ.path.is_empty() && typ.name == "Self" {
            return own.cloned();
        }
        if typ.path.iter().any(|segment| segment == "Self") {
            return None;
        }
        self.resolve_type(context, &typ.path, &typ.name)
    }

    /// The value a written type holds, its `Option` / `Result` layers kept.
    fn value(&self, context: &str, own: Option<&Type>, typ: &TypeRef) -> Option<Value> {
        self.resolve_ref(context, own, typ).map(|resolved| Value {
            typ: resolved,
            layers: typ.layers,
        })
    }

    fn lookup(&self, module: &str, typ: &str, name: &str) -> Vec<usize> {
        self.by_key
            .get(&(module.to_owned(), typ.to_owned(), name.to_owned()))
            .cloned()
            .unwrap_or_default()
    }

    /// The (module, type, fn) a path call names from `scope`; the type is empty for a free
    /// fn. A bare name is an explicit import, else a fn of the module, else the one glob
    /// import that has it, as Rust resolves it.
    fn path_target(
        &self,
        scope: &Scope,
        path: &[String],
        name: &str,
    ) -> Option<(String, String, String)> {
        let context = scope.context;
        match path.split_last() {
            None => {
                match self.imports.get(context).and_then(|names| names.get(name)) {
                    Some(Import::Item(module, item)) => {
                        return Some((module.clone(), String::new(), item.clone()));
                    }
                    Some(Import::Module(_)) => return None,
                    None => {}
                }
                if !self.lookup(context, "", name).is_empty() {
                    return Some((context.to_owned(), String::new(), name.to_owned()));
                }
                let globs: Vec<&String> = self
                    .globs
                    .get(context)
                    .into_iter()
                    .flatten()
                    .filter(|glob| !self.lookup(glob, "", name).is_empty())
                    .collect();
                match globs.as_slice() {
                    [glob] => Some(((*glob).clone(), String::new(), name.to_owned())),
                    _ => None,
                }
            }
            Some((last, prefix)) if prefix.is_empty() && last == "Self" => scope
                .own_type()
                .map(|(module, typ)| (module, typ, name.to_owned())),
            Some((last, prefix)) if is_type_name(last) => self
                .resolve_type(context, prefix, last)
                .map(|(module, typ)| (module, typ, name.to_owned())),
            Some(_) => self
                .resolve_module(context, path)
                .map(|module| (module, String::new(), name.to_owned())),
        }
    }

    /// The one value the fns `found` (looked up in `module`) return; `Self` is `own`.
    fn returned(&self, found: &[usize], module: &str, own: Option<&Type>) -> Option<Value> {
        let mut values = BTreeSet::new();
        for at in found {
            let ret = self.nodes[*at].ret.as_ref()?;
            values.insert(self.value(module, own, ret)?);
        }
        let mut values = values.into_iter();
        let value = values.next()?;
        values.next().is_none().then_some(value)
    }

    /// The value of the postfix expression that ends at token `end`, and its first token:
    /// `self`, a typed binding, a unit-struct value, a struct literal, a call whose declared
    /// return type resolves, a field, or a std `?`, `.unwrap()`, `.expect(..)` or `.clone()`.
    /// A declared method wins over the std name. A method call or a field on a value still
    /// wrapped in `Option` / `Result` is unresolved.
    fn chain(&self, scope: &Scope, end: usize, depth: usize) -> Option<(Value, usize)> {
        if depth > CHAIN_DEPTH {
            return None;
        }
        let lexed = scope.lexed;
        let plain = |typ: Type| Value { typ, layers: 0 };
        match lexed.tokens.get(end)?.kind {
            Kind::Punct(b'?') => {
                let (value, start) = self.chain(scope, end.checked_sub(1)?, depth + 1)?;
                let layers = value.layers.checked_sub(1)?;
                Some((Value { layers, ..value }, start))
            }
            Kind::Punct(b')') => {
                let open = opening(lexed, end)?;
                let callee = open.checked_sub(1)?;
                let name = lexed.ident(callee)?;
                if callee >= 1 && lexed.is_punct(callee - 1, b'.') {
                    let (receiver, start) = self.chain(scope, callee.checked_sub(2)?, depth + 1)?;
                    if receiver.layers == 0 {
                        let (module, typ) = &receiver.typ;
                        let found = self.lookup(module, typ, name);
                        if found.is_empty() {
                            return (name == "clone").then_some((receiver, start));
                        }
                        let value = self.returned(&found, module, Some(&receiver.typ))?;
                        return Some((value, start));
                    }
                    let layers = match name {
                        "unwrap" | "expect" => receiver.layers - 1,
                        "clone" => receiver.layers,
                        _ => return None,
                    };
                    return Some((Value { layers, ..receiver }, start));
                }
                let path = path_before(lexed, callee);
                let start = callee - path.len() * 3;
                if path.is_empty() && is_type_name(name) {
                    let typ = self.resolve_type(scope.context, &[], name)?;
                    return Some((plain(typ), start));
                }
                let (module, typ, item) = self.path_target(scope, &path, name)?;
                let own = (!typ.is_empty()).then(|| (module.clone(), typ.clone()));
                let found = self.lookup(&module, &typ, &item);
                let value = self.returned(&found, &module, own.as_ref())?;
                Some((value, start))
            }
            Kind::Punct(b'}') => {
                let open = opening(lexed, end)?;
                let name_at = open.checked_sub(1)?;
                let name = lexed.ident(name_at)?;
                let path = path_before(lexed, name_at);
                let typ = self.named_type(scope, &path, name)?;
                Some((plain(typ), name_at - path.len() * 3))
            }
            Kind::Ident => {
                let name = lexed.ident(end)?;
                if end >= 1 && lexed.is_punct(end - 1, b'.') {
                    let (base, start) = self.chain(scope, end.checked_sub(2)?, depth + 1)?;
                    if base.layers != 0 {
                        return None;
                    }
                    let field = self.fields.get(&base.typ)?.get(name)?;
                    let value = self.value(&base.typ.0, Some(&base.typ), field)?;
                    return Some((value, start));
                }
                let path = path_before(lexed, end);
                let start = end - path.len() * 3;
                if path.is_empty() && name == "self" {
                    return scope.own_type().map(|typ| (plain(typ), start));
                }
                if is_type_name(name) {
                    return self
                        .named_type(scope, &path, name)
                        .map(|typ| (plain(typ), start));
                }
                if !path.is_empty() {
                    return None;
                }
                let value = self.bound_value(scope, name, end, depth + 1)?;
                Some((value, start))
            }
            _ => None,
        }
    }

    /// The type a type name or path names in an expression; `Self` is the fn's own type.
    fn named_type(&self, scope: &Scope, path: &[String], name: &str) -> Option<Type> {
        if !is_type_name(name) {
            return None;
        }
        if path.is_empty() && name == "Self" {
            return scope.own_type();
        }
        if path.iter().any(|segment| segment == "Self") {
            return None;
        }
        self.resolve_type(scope.context, path, name)
    }

    /// The value of the binding `name` that is in scope at token `at`.
    fn bound_value(&self, scope: &Scope, name: &str, at: usize, depth: usize) -> Option<Value> {
        let binding = scope
            .bindings
            .iter()
            .filter(|binding| binding.name == name && binding.from < at && at < binding.until)
            .max_by_key(|binding| binding.from)?;
        match &binding.bound {
            Bound::Typed(typ) => self.value(scope.context, scope.own_type().as_ref(), typ),
            Bound::Init(start, end) => {
                let (typ, first) = self.chain(scope, end.checked_sub(1)?, depth)?;
                let mut lead = *start;
                while lead < first
                    && (scope.lexed.is_punct(lead, b'&') || scope.lexed.is_ident(lead, "mut"))
                {
                    lead += 1;
                }
                (lead == first).then_some(typ)
            }
            Bound::Opaque => None,
        }
    }

    /// Every call of fn `node`, constructions that reach a `Drop` included.
    fn callees(&self, node: usize, referenced: &TypeSet) -> Edges {
        let mut edges = Edges::default();
        let Some(this) = self.nodes.get(node) else {
            return edges;
        };
        let Some(lexed) = self.lexed.get(&this.file) else {
            return edges;
        };
        let bindings = bindings(lexed, this);
        for context in &this.contexts {
            let scope = Scope {
                node: this,
                context,
                lexed,
                bindings: &bindings,
            };
            for at in this.body.0 + 1..this.body.1 {
                self.calls_at(&scope, at, referenced, &mut edges);
                self.drops_at(&scope, at, &mut edges.proven);
            }
        }
        edges.proven.remove(&node);
        edges.may.remove(&node);
        let proven = &edges.proven;
        edges.may.retain(|callee| !proven.contains(callee));
        edges
    }

    /// The (module, type) pairs a fn body may hold a value of: types named in its file,
    /// resolved from each of the fn's module contexts.
    fn referenced_types(&self, file: &str, contexts: &[String]) -> TypeSet {
        let mut found = BTreeSet::new();
        let Some(lexed) = self.lexed.get(file) else {
            return found;
        };
        for at in 0..lexed.tokens.len() {
            let Some(name) = lexed.ident(at) else {
                continue;
            };
            if !name.starts_with(|first: char| first.is_ascii_uppercase()) {
                continue;
            }
            let path = path_before(lexed, at);
            for context in contexts {
                found.extend(self.resolve_type(context, &path, name));
            }
        }
        found
    }

    fn calls_at(&self, scope: &Scope, at: usize, referenced: &TypeSet, edges: &mut Edges) {
        let lexed = scope.lexed;
        let Some(name) = lexed.ident(at) else { return };
        if !is_call(lexed, at) {
            return;
        }
        let path = path_before(lexed, at);
        let start = at - path.len() * 3;
        if path.is_empty() && start > 0 && lexed.is_punct(start - 1, b'.') {
            let proven = start
                .checked_sub(2)
                .and_then(|end| self.chain(scope, end, 0))
                .filter(|(receiver, _)| receiver.layers == 0)
                .map(|(receiver, _)| self.lookup(&receiver.typ.0, &receiver.typ.1, name))
                .filter(|found| !found.is_empty());
            match proven {
                Some(found) => edges.proven.extend(found),
                None => {
                    for candidate in self.methods.get(name).into_iter().flatten() {
                        let node = &self.nodes[*candidate];
                        if node
                            .contexts
                            .iter()
                            .any(|module| referenced.contains(&(module.clone(), node.typ.clone())))
                        {
                            edges.may.insert(*candidate);
                        }
                    }
                }
            }
            return;
        }
        if start > 0 && (lexed.is_ident(start - 1, "fn") || lexed.is_punct(start - 1, b'.')) {
            return;
        }
        if let Some((module, typ, item)) = self.path_target(scope, &path, name) {
            edges.proven.extend(self.lookup(&module, &typ, &item));
        }
    }

    /// A construction of a type with a `Drop` impl: `T { .. }`, or `T::f(..)` / `Self::f(..)`
    /// where `f` returns `Self` or `T`. Merely naming the type is not one.
    fn drops_at(&self, scope: &Scope, at: usize, out: &mut BTreeSet<usize>) {
        let lexed = scope.lexed;
        let Some(name) = lexed.ident(at) else { return };
        if !name.starts_with(|first: char| first.is_ascii_uppercase()) {
            return;
        }
        let path = path_before(lexed, at);
        let start = at - path.len() * 3;
        let before = start.checked_sub(1).and_then(|prior| lexed.ident(prior));
        if matches!(
            before,
            Some("struct" | "enum" | "impl" | "for" | "union" | "trait")
        ) {
            return;
        }
        let target = if name == "Self" {
            scope.own_type()
        } else {
            self.resolve_type(scope.context, &path, name)
        };
        let Some(target) = target else { return };
        let Some(drop) = self.drops.get(&target) else {
            return;
        };
        let literal = name != "Self" && lexed.is_punct(at + 1, b'{');
        let built = lexed.is_path_sep(at + 1)
            && lexed.ident(at + 3).is_some_and(|method| {
                is_call(lexed, at + 3)
                    && self
                        .lookup(&target.0, &target.1, method)
                        .iter()
                        .any(|candidate| self.nodes[*candidate].returns_self)
            });
        if literal || built {
            out.insert(*drop);
        }
    }

    fn reaches_primitive(&self, node: &Node) -> bool {
        let Some(lexed) = self.lexed.get(&node.file) else {
            return false;
        };
        (node.body.0..node.body.1).any(|at| {
            (lexed.is_ident(at, "Command")
                && lexed.is_path_sep(at + 1)
                && lexed.is_ident(at + 3, "new"))
                || binary_path(lexed, at)
                || lexed.is_ident(at, "CARGO_BIN_EXE_ae")
                || (lexed.is_punct(at, b'.')
                    && lexed.is_ident(at + 1, "spawn")
                    && lexed.is_punct(at + 2, b'(')
                    && lexed.is_punct(at + 3, b')'))
        })
    }

    /// Every shared boundary helper. Discovery follows proven and may edges, so an untyped
    /// receiver widens the helper set; the driving tests follow proven edges only, so a
    /// helper no test provably drives keeps an empty caller list and no member can pass.
    fn helpers(&self) -> Vec<Helper> {
        // One scan of a file's type names per module context, shared by its fns.
        let mut referenced: BTreeMap<(&str, &[String]), TypeSet> = BTreeMap::new();
        let edges: Vec<Edges> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(at, node)| {
                let types = referenced
                    .entry((node.file.as_str(), node.contexts.as_slice()))
                    .or_insert_with(|| self.referenced_types(&node.file, &node.contexts));
                self.callees(at, types)
            })
            .collect();
        let mut reach: BTreeSet<usize> = (0..self.nodes.len())
            .filter(|node| !self.nodes[*node].test && self.reaches_primitive(&self.nodes[*node]))
            .collect();
        loop {
            let grown: Vec<usize> = (0..self.nodes.len())
                .filter(|node| !self.nodes[*node].test && !reach.contains(node))
                .filter(|node| {
                    let calls = &edges[*node];
                    calls
                        .proven
                        .iter()
                        .chain(&calls.may)
                        .any(|callee| reach.contains(callee))
                })
                .collect();
            if grown.is_empty() {
                break;
            }
            reach.extend(grown);
        }
        let mut callers: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); self.nodes.len()];
        let mut proven: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); self.nodes.len()];
        for (caller, calls) in edges.iter().enumerate() {
            for callee in &calls.proven {
                callers[*callee].insert(caller);
                proven[*callee].insert(caller);
            }
            for callee in &calls.may {
                callers[*callee].insert(caller);
            }
        }
        let mut merged: BTreeMap<(String, String), Helper> = BTreeMap::new();
        for helper in &reach {
            let node = &self.nodes[*helper];
            if !callers[*helper]
                .iter()
                .any(|caller| self.nodes[*caller].file != node.file)
            {
                continue;
            }
            let item = if node.typ.is_empty() {
                node.name.clone()
            } else {
                format!("{}::{}", node.typ, node.name)
            };
            let tests = self.driving_tests(*helper, &proven);
            let entry = merged
                .entry((node.file.clone(), item.clone()))
                .or_insert_with(|| Helper {
                    file: node.file.clone(),
                    item,
                    line: node.line,
                    callers: Vec::new(),
                });
            entry.callers.extend(tests);
        }
        merged
            .into_values()
            .map(|mut helper| {
                helper.callers.sort();
                helper.callers.dedup();
                helper
            })
            .collect()
    }

    /// The full names of the tests that provably reach `helper` in a normal run (child
    /// modes excluded); `callers` holds each fn's proven callers.
    fn driving_tests(&self, helper: usize, callers: &[BTreeSet<usize>]) -> Vec<String> {
        let mut seen = BTreeSet::from([helper]);
        let mut queue = VecDeque::from([helper]);
        let mut tests = Vec::new();
        while let Some(at) = queue.pop_front() {
            let node = &self.nodes[at];
            if node.test && !node.child_mode {
                tests.extend(
                    node.contexts
                        .iter()
                        .map(|context| join_path(context, &node.name)),
                );
            }
            for caller in &callers[at] {
                if seen.insert(*caller) {
                    queue.push_back(*caller);
                }
            }
        }
        tests
    }
}

/// The product binary's path read at `at`: `env!("CARGO_BIN_EXE_ae")`, `option_env!(..)`,
/// `var(..)` or `var_os(..)`. The name as a plain string (this guard's own source) is not one.
fn binary_path(lexed: &Lexed, at: usize) -> bool {
    lexed.literal(at) == Some("CARGO_BIN_EXE_ae")
        && at >= 2
        && lexed.is_punct(at - 1, b'(')
        && ((lexed.is_punct(at - 2, b'!')
            && at >= 3
            && (lexed.is_ident(at - 3, "env") || lexed.is_ident(at - 3, "option_env")))
            || lexed.is_ident(at - 2, "var")
            || lexed.is_ident(at - 2, "var_os"))
}

/// A fn read from its `fn` keyword.
struct FnItem {
    typ: String,
    name: String,
    test: bool,
    /// The body's open and close brace.
    body: (usize, usize),
    /// Declared to return `Self` or its own type, unwrapped.
    returns_self: bool,
    /// The byte offset of the `fn` keyword.
    start: usize,
    params: Vec<(String, Option<TypeRef>)>,
    ret: Option<TypeRef>,
}

/// The fn whose `fn` keyword is at `at`, if it has a body.
fn read_fn(
    lexed: &Lexed,
    at: usize,
    attrs: &[(usize, usize)],
    impls: &[(usize, usize, String)],
) -> Option<FnItem> {
    let name = lexed.ident(at + 1)?.to_owned();
    let mut open = at + 2;
    let mut depth = 0usize;
    let mut angle = 0usize;
    let mut list = None;
    loop {
        match lexed.tokens.get(open)?.kind {
            Kind::Punct(b'(') => {
                if depth == 0 && angle == 0 && list.is_none() {
                    list = Some(open);
                }
                depth += 1;
            }
            Kind::Punct(b'[') => depth += 1,
            Kind::Punct(b')' | b']') => depth = depth.saturating_sub(1),
            Kind::Punct(b'<') if depth == 0 => angle += 1,
            Kind::Punct(b'>') if depth == 0 && !lexed.is_punct(open - 1, b'-') => {
                angle = angle.saturating_sub(1);
            }
            Kind::Punct(b';') if depth == 0 => return None,
            Kind::Punct(b'{') if depth == 0 => break,
            _ => {}
        }
        open += 1;
    }
    let close = lexed.matching(open)?;
    let test = attrs
        .iter()
        .any(|(start, end)| *end == start + 3 && lexed.is_ident(start + 2, "test"));
    let typ = impls
        .iter()
        .filter(|(body_open, body_close, _)| *body_open < at && at < *body_close)
        .min_by_key(|(body_open, body_close, _)| body_close - body_open)
        .map(|(_, _, typ)| typ.clone())
        .unwrap_or_default();
    let returns_self = (at + 2..open).any(|arrow| {
        lexed.is_punct(arrow, b'-')
            && lexed.is_punct(arrow + 1, b'>')
            && lexed
                .ident(arrow + 2)
                .is_some_and(|returned| returned == "Self" || (!typ.is_empty() && returned == typ))
    });
    let mut params = Vec::new();
    let mut ret = None;
    if let Some(list) = list
        && let Some(list_close) = lexed.matching(list)
    {
        params = read_params(lexed, list, list_close);
        if lexed.is_punct(list_close + 1, b'-') && lexed.is_punct(list_close + 2, b'>') {
            let stop = (list_close + 3..open)
                .find(|index| lexed.is_ident(*index, "where"))
                .unwrap_or(open);
            ret = type_ref(lexed, list_close + 3, stop);
        }
    }
    Some(FnItem {
        typ,
        name,
        test,
        body: (open, close),
        returns_self,
        start: lexed.tokens[at].start,
        params,
        ret,
    })
}

/// The named parameters of the list `open..close`: `self` skipped, the names of any
/// other pattern untyped.
fn read_params(lexed: &Lexed, open: usize, close: usize) -> Vec<(String, Option<TypeRef>)> {
    let mut params = Vec::new();
    for (start, end) in split_commas(lexed, open + 1, close) {
        let colon = (start..end).find(|index| is_colon(lexed, *index));
        params.extend(pattern(lexed, start, colon.unwrap_or(end), |name| {
            colon
                .and_then(|colon| type_ref(lexed, colon + 1, end))
                .map(|typ| (name, typ))
        }));
    }
    params
}

/// The names a pattern in tokens `start..end` binds. A plain `name` or `mut name` gets
/// `typed(name)`; every name of any other pattern is untyped.
fn pattern<F>(lexed: &Lexed, start: usize, end: usize, typed: F) -> Vec<(String, Option<TypeRef>)>
where
    F: FnOnce(String) -> Option<(String, TypeRef)>,
{
    let names: Vec<String> = (start..end)
        .filter_map(|index| lexed.ident(index))
        .filter(|name| !matches!(*name, "mut" | "ref" | "self"))
        .map(str::to_owned)
        .collect();
    let plain = (start..end).all(|index| lexed.ident(index).is_some())
        && !(start..end).any(|index| lexed.is_ident(index, "self"));
    match names.as_slice() {
        [name] if plain => match typed(name.clone()) {
            Some((name, typ)) => vec![(name, Some(typ))],
            None => vec![(name.clone(), None)],
        },
        _ => names.into_iter().map(|name| (name, None)).collect(),
    }
}

/// The comma-separated items of tokens `start..end`, at bracket depth 0.
fn split_commas(lexed: &Lexed, start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut item = start;
    for at in start..end {
        match lexed.tokens[at].kind {
            Kind::Punct(b'(' | b'[' | b'{' | b'<') => depth += 1,
            Kind::Punct(b'>') if lexed.is_punct(at - 1, b'-') => {}
            Kind::Punct(b')' | b']' | b'}' | b'>') => depth = depth.saturating_sub(1),
            Kind::Punct(b',') if depth == 0 => {
                if item < at {
                    items.push((item, at));
                }
                item = at + 1;
            }
            _ => {}
        }
    }
    if item < end {
        items.push((item, end));
    }
    items
}

/// A single `:`, not half of `::`.
fn is_colon(lexed: &Lexed, at: usize) -> bool {
    lexed.is_punct(at, b':') && !lexed.is_path_sep(at) && !(at > 0 && lexed.is_path_sep(at - 1))
}

/// An uppercase-initial name: a type, a variant or `Self`.
fn is_type_name(name: &str) -> bool {
    name.starts_with(|first: char| first.is_ascii_uppercase())
}

/// The type written in tokens `start..end`, references, lifetimes and `Box` / `Arc` /
/// `Rc` peeled, and `Option` / `Result` peeled and counted as layers; `None` for tuples,
/// slices, `impl` and `dyn`.
fn type_ref(lexed: &Lexed, start: usize, end: usize) -> Option<TypeRef> {
    let mut at = start;
    while at < end
        && (lexed.is_punct(at, b'&')
            || lexed.is_ident(at, "mut")
            || lexed.tokens.get(at)?.kind == Kind::Other)
    {
        at += 1;
    }
    if at >= end {
        return None;
    }
    let mut name = lexed.ident(at)?.to_owned();
    if matches!(name.as_str(), "impl" | "dyn") {
        return None;
    }
    let mut path = Vec::new();
    at += 1;
    while at + 2 < end && lexed.is_path_sep(at) {
        let next = lexed.ident(at + 2)?.to_owned();
        path.push(std::mem::replace(&mut name, next));
        at += 3;
    }
    if matches!(name.as_str(), "Result" | "Option" | "Box" | "Arc" | "Rc")
        && at < end
        && lexed.is_punct(at, b'<')
    {
        let (first, _) = split_commas(lexed, at + 1, end).into_iter().next()?;
        let mut depth = 0usize;
        let mut last = first;
        while last < end {
            if lexed.is_punct(last, b'<') {
                depth += 1;
            } else if lexed.is_punct(last, b'>') && !lexed.is_punct(last - 1, b'-') {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            } else if lexed.is_punct(last, b',') && depth == 0 {
                break;
            }
            last += 1;
        }
        let mut inner = type_ref(lexed, first, last)?;
        if matches!(name.as_str(), "Result" | "Option") {
            inner.layers += 1;
        }
        return Some(inner);
    }
    Some(TypeRef {
        path,
        name,
        layers: 0,
    })
}

/// The named fields of the struct whose name is at `name_at`, by field name.
fn struct_fields(lexed: &Lexed, name_at: usize) -> BTreeMap<String, TypeRef> {
    let mut fields = BTreeMap::new();
    let mut angle = 0usize;
    let mut open = name_at + 1;
    while open < lexed.tokens.len() {
        if lexed.is_punct(open, b'<') {
            angle += 1;
        } else if lexed.is_punct(open, b'>') && !lexed.is_punct(open - 1, b'-') {
            angle = angle.saturating_sub(1);
        } else if angle == 0
            && (lexed.is_punct(open, b'{')
                || lexed.is_punct(open, b'(')
                || lexed.is_punct(open, b';'))
        {
            break;
        }
        open += 1;
    }
    if !lexed.is_punct(open, b'{') {
        return fields;
    }
    let Some(close) = lexed.matching(open) else {
        return fields;
    };
    for (start, end) in split_commas(lexed, open + 1, close) {
        let mut at = start;
        while let Some(attribute) = lexed.attribute_end(at) {
            at = attribute + 1;
        }
        if lexed.is_ident(at, "pub") {
            at = lexed.skip_visibility(at);
        }
        if let Some(name) = lexed.ident(at)
            && is_colon(lexed, at + 1)
            && let Some(typ) = type_ref(lexed, at + 2, end)
        {
            fields.insert(name.to_owned(), typ);
        }
    }
    fields
}

/// The index of the bracket that opens the one at `close` (`)`, `]` or `}`).
fn opening(lexed: &Lexed, close: usize) -> Option<usize> {
    let (left, right) = match lexed.tokens.get(close)?.kind {
        Kind::Punct(b')') => (b'(', b')'),
        Kind::Punct(b']') => (b'[', b']'),
        Kind::Punct(b'}') => (b'{', b'}'),
        _ => return None,
    };
    let mut depth = 0usize;
    for index in (0..=close).rev() {
        if lexed.is_punct(index, right) {
            depth += 1;
        } else if lexed.is_punct(index, left) {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

/// The close brace of the innermost block of `from..` that holds token `at`.
fn enclosing_block(lexed: &Lexed, from: usize, at: usize) -> Option<usize> {
    let mut depth = 0usize;
    for index in (from..at).rev() {
        if lexed.is_punct(index, b'}') {
            depth += 1;
        } else if lexed.is_punct(index, b'{') {
            if depth == 0 {
                return lexed.matching(index);
            }
            depth -= 1;
        }
    }
    None
}

/// Every name fn `node` binds: its parameters, `let` statements, and, untyped, the names
/// of closure heads, `for` patterns, `if let` / `while let` patterns, match arms and the
/// parameters of nested fns. A name rebound by a pattern this guard does not type is
/// unresolved from there to the end of the enclosing block.
fn bindings(lexed: &Lexed, node: &Node) -> Vec<Binding> {
    let (open, close) = node.body;
    let mut found: Vec<Binding> = node
        .params
        .iter()
        .map(|(name, typ)| Binding {
            name: name.clone(),
            from: open,
            until: close,
            bound: typ.clone().map_or(Bound::Opaque, Bound::Typed),
        })
        .collect();
    let block = |at: usize| enclosing_block(lexed, open, at).unwrap_or(close);
    let opaque = |found: &mut Vec<Binding>, start: usize, end: usize, from: usize, until: usize| {
        found.extend(
            (start..end)
                .filter_map(|index| lexed.ident(index))
                .map(|name| Binding {
                    name: name.to_owned(),
                    from,
                    until,
                    bound: Bound::Opaque,
                }),
        );
    };
    for at in open + 1..close {
        if lexed.is_punct(at, b'|') && closure_head(lexed, at) {
            if let Some(end) = (at + 1..close).find(|index| lexed.is_punct(*index, b'|')) {
                opaque(&mut found, at + 1, end, at, block(at));
            }
            continue;
        }
        match lexed.ident(at) {
            Some("let") => found.extend(let_binding(lexed, at, close, block(at))),
            Some("for") => {
                if let Some(end) = (at + 1..close).find(|index| lexed.is_ident(*index, "in")) {
                    opaque(&mut found, at + 1, end, at, block(at));
                }
            }
            Some("match") => match_arms(lexed, at, close, &mut found),
            Some("fn") => {
                if let Some(item) = read_fn(lexed, at, &[], &[]) {
                    found.extend(item.params.into_iter().map(|(name, _)| Binding {
                        name,
                        from: at,
                        until: item.body.1,
                        bound: Bound::Opaque,
                    }));
                }
            }
            _ => {}
        }
    }
    found
}

/// Whether the `|` at `at` opens a closure head: nothing that ends an operand comes before it.
fn closure_head(lexed: &Lexed, at: usize) -> bool {
    let Some(prior) = at.checked_sub(1) else {
        return true;
    };
    let operand = match lexed.tokens[prior].kind {
        Kind::Ident => !matches!(
            lexed.ident(prior),
            Some("move" | "return" | "in" | "if" | "else" | "match" | "while" | "let" | "mut")
        ),
        Kind::Literal(_) | Kind::Other => true,
        Kind::Punct(byte) => matches!(byte, b')' | b']' | b'}' | b'?' | b'|'),
    };
    !operand
}

/// The names the `let` at `at` binds, its statement ending before `limit`; in scope until `until`.
fn let_binding(lexed: &Lexed, at: usize, limit: usize, until: usize) -> Vec<Binding> {
    let conditional = at > 0
        && (lexed.is_ident(at - 1, "if")
            || lexed.is_ident(at - 1, "while")
            || lexed.is_punct(at - 1, b'&')
            || lexed.is_punct(at - 1, b'|'));
    let mut depth = 0usize;
    let mut end = at + 1;
    while end < limit {
        match lexed.tokens[end].kind {
            Kind::Punct(b'(' | b'[' | b'{') => depth += 1,
            Kind::Punct(b')' | b']' | b'}') => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            Kind::Punct(b':') if depth == 0 && is_colon(lexed, end) => break,
            Kind::Punct(b'=')
                if depth == 0
                    && !lexed.is_punct(end + 1, b'=')
                    && !lexed.is_punct(end + 1, b'>') =>
            {
                break;
            }
            Kind::Punct(b';') if depth == 0 => break,
            _ => {}
        }
        end += 1;
    }
    let names = pattern(lexed, at + 1, end, |_| None);
    let opaque = |names: Vec<(String, Option<TypeRef>)>| {
        names
            .into_iter()
            .map(|(name, _)| Binding {
                name,
                from: at,
                until,
                bound: Bound::Opaque,
            })
            .collect()
    };
    let plain = names.len() == 1 && (at + 1..end).all(|index| lexed.ident(index).is_some());
    if conditional || !plain {
        return opaque(names);
    }
    let (annotated, eq) = if is_colon(lexed, end) {
        let eq = statement_end(lexed, end + 1, limit, true);
        (Some(type_ref(lexed, end + 1, eq)), eq)
    } else {
        (None, end)
    };
    let semicolon = statement_end(lexed, eq, limit, false);
    let initialized = lexed.is_punct(eq, b'=');
    let else_branch = (eq..semicolon).any(|index| lexed.is_ident(index, "else"))
        && !(eq..semicolon).any(|index| lexed.is_ident(index, "if"));
    let bound = match annotated {
        Some(Some(typ)) => Bound::Typed(typ),
        None if initialized && !else_branch => Bound::Init(eq + 1, semicolon),
        Some(None) | None => Bound::Opaque,
    };
    names
        .into_iter()
        .next()
        .map(|(name, _)| Binding {
            name,
            from: semicolon,
            until,
            bound,
        })
        .into_iter()
        .collect()
}

/// The first depth-0 `;` (or, with `or_assign`, `=`) at or after `start`, else `limit`.
fn statement_end(lexed: &Lexed, start: usize, limit: usize, or_assign: bool) -> usize {
    let mut depth = 0usize;
    let mut angle = 0usize;
    for at in start..limit {
        match lexed.tokens[at].kind {
            Kind::Punct(b'(' | b'[' | b'{') => depth += 1,
            Kind::Punct(b')' | b']' | b'}') => depth = depth.saturating_sub(1),
            Kind::Punct(b'<') if or_assign => angle += 1,
            Kind::Punct(b'>') if or_assign && !lexed.is_punct(at - 1, b'-') => {
                angle = angle.saturating_sub(1);
            }
            Kind::Punct(b';') if depth == 0 => return at,
            Kind::Punct(b'=')
                if or_assign && depth == 0 && angle == 0 && !lexed.is_punct(at + 1, b'=') =>
            {
                return at;
            }
            _ => {}
        }
    }
    limit
}

/// The names the arm patterns of the `match` at `at` bind, in scope to the match's end.
fn match_arms(lexed: &Lexed, at: usize, limit: usize, found: &mut Vec<Binding>) {
    let mut depth = 0usize;
    let Some(open) = (at + 1..limit).find(|index| {
        match lexed.tokens[*index].kind {
            Kind::Punct(b'(' | b'[') => depth += 1,
            Kind::Punct(b')' | b']') => depth = depth.saturating_sub(1),
            _ => {}
        }
        depth == 0 && lexed.is_punct(*index, b'{')
    }) else {
        return;
    };
    let Some(close) = lexed.matching(open) else {
        return;
    };
    let mut in_pattern = true;
    let mut depth = 0usize;
    let mut block_body = false;
    let mut cursor = open + 1;
    while cursor < close {
        if in_pattern {
            if lexed.is_punct(cursor, b'=') && lexed.is_punct(cursor + 1, b'>') {
                in_pattern = false;
                block_body = lexed.is_punct(cursor + 2, b'{');
                depth = 0;
                cursor += 2;
                continue;
            }
            if let Some(name) = lexed.ident(cursor) {
                found.push(Binding {
                    name: name.to_owned(),
                    from: cursor,
                    until: close,
                    bound: Bound::Opaque,
                });
            }
        } else {
            match lexed.tokens[cursor].kind {
                Kind::Punct(b'(' | b'[' | b'{') => depth += 1,
                Kind::Punct(closer @ (b')' | b']' | b'}')) => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 && closer == b'}' && block_body {
                        in_pattern = true;
                    }
                }
                Kind::Punct(b',') if depth == 0 => in_pattern = true,
                _ => {}
            }
        }
        cursor += 1;
    }
}

/// Every `impl ... {}` block: (open brace, close brace, implemented type).
fn impl_blocks(lexed: &Lexed) -> Vec<(usize, usize, String)> {
    let mut blocks = Vec::new();
    for at in 0..lexed.tokens.len() {
        if !lexed.is_ident(at, "impl") {
            continue;
        }
        let mut names: Vec<&str> = Vec::new();
        let mut angle = 0usize;
        let mut open = at + 1;
        while open < lexed.tokens.len() && !(angle == 0 && lexed.is_punct(open, b'{')) {
            if lexed.is_punct(open, b';') && angle == 0 {
                break;
            }
            if lexed.is_punct(open, b'<') {
                angle += 1;
            } else if lexed.is_punct(open, b'>') && !lexed.is_punct(open - 1, b'-') {
                angle = angle.saturating_sub(1);
            } else if angle == 0
                && let Some(name) = lexed.ident(open)
            {
                names.push(name);
            }
            open += 1;
        }
        if !lexed.is_punct(open, b'{') {
            continue;
        }
        let path: Vec<&str> = match names.iter().position(|name| *name == "for") {
            Some(split) => names[split + 1..].to_vec(),
            None => names,
        };
        let path: Vec<&str> = path
            .into_iter()
            .take_while(|name| *name != "where")
            .collect();
        if let (Some(typ), Some(close)) = (path.last(), lexed.matching(open)) {
            blocks.push((open, close, (*typ).to_owned()));
        }
    }
    blocks
}

/// A child-mode test returns early unless an environment variable is set, so it is a
/// no-op in a normal run: `let Some(..) = env::var_os(..) else { return; }`, or
/// `if env::var_os(..).is_none() { .. return; }` (`env::var` and `.is_err()` alike).
fn child_mode(lexed: &Lexed, body: (usize, usize)) -> bool {
    let end = body.1.min(body.0 + 80);
    let returns = |open: usize| {
        lexed.is_punct(open, b'{')
            && lexed
                .matching(open)
                .is_some_and(|close| (open + 1..close).any(|at| lexed.is_ident(at, "return")))
    };
    (body.0..end).any(|at| {
        (lexed.is_ident(at, "var") || lexed.is_ident(at, "var_os"))
            && at >= 3
            && lexed.is_path_sep(at - 2)
            && lexed.is_ident(at - 3, "env")
            && lexed.is_punct(at + 1, b'(')
            && lexed.matching(at + 1).is_some_and(|close| {
                (lexed.is_ident(close + 1, "else") && returns(close + 2))
                    || (lexed.is_punct(close + 1, b'.')
                        && (lexed.is_ident(close + 2, "is_none")
                            || lexed.is_ident(close + 2, "is_err"))
                        && lexed.is_punct(close + 3, b'(')
                        && lexed.is_punct(close + 4, b')')
                        && returns(close + 5))
            })
    })
}

/// Whether the identifier at `at` is called: `name(` or `name::<..>(`.
fn is_call(lexed: &Lexed, at: usize) -> bool {
    if lexed.is_punct(at + 1, b'(') {
        return true;
    }
    if lexed.is_path_sep(at + 1) && lexed.is_punct(at + 3, b'<') {
        let mut angle = 0usize;
        for next in at + 3..lexed.tokens.len().min(at + 64) {
            if lexed.is_punct(next, b'<') {
                angle += 1;
            } else if lexed.is_punct(next, b'>') && !lexed.is_punct(next - 1, b'-') {
                angle -= 1;
                if angle == 0 {
                    return lexed.is_punct(next + 1, b'(');
                }
            }
        }
    }
    false
}

/// The `a::b::` segments directly before the identifier at `at`, outermost first.
fn path_before(lexed: &Lexed, at: usize) -> Vec<String> {
    let mut path = Vec::new();
    let mut cursor = at;
    while cursor >= 3 && lexed.is_path_sep(cursor - 2) {
        let Some(segment) = lexed.ident(cursor - 3) else {
            break;
        };
        path.push(segment.to_owned());
        cursor -= 3;
    }
    path.reverse();
    path
}

fn parent_module(module: &str) -> Option<String> {
    if module.is_empty() {
        return None;
    }
    Some(
        module
            .rsplit_once("::")
            .map_or_else(String::new, |(parent, _)| parent.to_owned()),
    )
}

// ---------------------------------------------------------------------------
// The check
// ---------------------------------------------------------------------------

/// Every way `entries` fails to cover `files`; empty is a pass.
pub(crate) fn check(entries: &[Entry], files: &Files) -> Vec<Finding> {
    let tree = match modtree::tree(files) {
        Ok(tree) => tree,
        Err(error) => return vec![Finding::UnresolvedTree(error)],
    };
    let mut findings = Vec::new();
    let units = entries
        .iter()
        .filter(|entry| **entry == Entry::Unit)
        .count();
    if units != 1 {
        findings.push(Finding::UnitMissingOrRepeated);
    }
    let modules: BTreeSet<&str> = tree
        .modules
        .iter()
        .filter(|module| module.krate == Crate::It)
        .map(|module| module.path.as_str())
        .collect();
    let tests: BTreeSet<String> = test_names(files, &tree.owners).into_iter().collect();
    let known_helpers = helpers(files, &tree.owners);
    let mut named: BTreeSet<String> = BTreeSet::new();
    for entry in entries {
        match entry {
            Entry::Module(module) if !modules.contains(module.as_str()) => {
                findings.push(Finding::UnknownModule(module.clone()));
            }
            Entry::Test(test) if !tests.contains(test) => {
                findings.push(Finding::UnknownModule(test.clone()));
            }
            Entry::History { target, .. }
                if !modules.contains(target.as_str()) && !tests.contains(target) =>
            {
                findings.push(Finding::UnknownModule(target.clone()));
            }
            Entry::Helper { item, member } => {
                named.insert(item.clone());
                match known_helpers
                    .iter()
                    .find(|helper| format!("{}::{}", helper.file, helper.item) == *item)
                {
                    None => findings.push(Finding::StaleHelper(item.clone())),
                    Some(helper) if !helper.callers.contains(member) => {
                        findings.push(Finding::MemberNotCaller {
                            item: item.clone(),
                            member: member.clone(),
                        });
                    }
                    Some(_) => {}
                }
            }
            Entry::Unit | Entry::Module(_) | Entry::Test(_) | Entry::History { .. } => {}
        }
    }
    let covered = |site: &Site| match site.module.krate {
        Crate::Lib | Crate::Bin => units > 0,
        Crate::It => entries.iter().any(|entry| match entry {
            Entry::Module(module) | Entry::History { target: module, .. } => {
                site.module.path == *module || site.module.path.starts_with(&format!("{module}::"))
            }
            _ => false,
        }),
    };
    let (group_sites, group_findings) = client_group(files);
    findings.extend(group_findings);
    for site in &group_sites {
        if !modules.contains(site.module.path.as_str()) {
            findings.push(Finding::UnknownModule(site.module.path.clone()));
        }
    }
    findings.extend(
        sites(files, &tree.owners)
            .into_iter()
            .filter(|site| !covered(site))
            .map(Finding::UncoveredSite),
    );
    findings.extend(
        known_helpers
            .into_iter()
            .filter(|helper| !named.contains(&format!("{}::{}", helper.file, helper.item)))
            .map(Finding::HelperWithoutMember),
    );
    findings
}

// The supplemental acceptance controls: receiver typing and quoted TOML keys, then
// declared methods before std names and wrapped receivers, then the env-gated child form.
#[allow(
    clippy::expect_used,
    reason = "the frozen spec uses expect in its fixtures and its bytes stay as nav froze them"
)]
mod chain_spec;
mod child_spec;
mod resolver_spec;
