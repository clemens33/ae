//! The module tree of ae's three crates, read from source text alone.
//!
//! The library (`src/lib.rs`), the binary (`src/main.rs`) and the integration
//! target (`tests/it/main.rs`) are walked from their roots through every
//! out-of-line `mod`, `#[path]` attribute, inline `mod { }` block and literal
//! `include!`. Every file maps to EVERY module path that owns it: a file reached
//! twice (two `#[path]` declarations, or once from the library and once from the
//! integration target) has two owners. A `mod` whose file is absent is an error,
//! never a guess: a guard that cannot place a file fails closed.
//!
//! Pure: the caller reads the files. No process, no filesystem, no environment.
//! The lexer is shared with the Mac subset guard (`macos_subset.rs`).

use std::collections::{BTreeMap, BTreeSet};

/// Repository-relative path (forward slashes) to the file's text.
pub(crate) type Files = BTreeMap<String, String>;

/// The crate a module path lives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Crate {
    Lib,
    Bin,
    It,
}

/// One module path inside one crate; `""` is the crate root.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ModPath {
    pub(crate) krate: Crate,
    pub(crate) path: String,
}

/// Why the tree could not be walked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TreeError {
    /// A `mod` declaration (or a literal `include!`) names no file in the set.
    Unresolved {
        file: String,
        line: usize,
        module: String,
    },
    /// A crate root is missing from the set.
    Root(String),
}

/// The three crate roots, in crate order.
pub(crate) const ROOTS: [(&str, Crate); 3] = [
    ("src/lib.rs", Crate::Lib),
    ("src/main.rs", Crate::Bin),
    ("tests/it/main.rs", Crate::It),
];

/// The walked tree: every file's owners and every module path, inline ones included.
#[derive(Clone, Debug, Default)]
pub(crate) struct Tree {
    pub(crate) owners: BTreeMap<String, Vec<ModPath>>,
    pub(crate) modules: BTreeSet<ModPath>,
}

/// Every file reached from the three roots, mapped to every module path that owns it, sorted.
pub(crate) fn module_map(files: &Files) -> Result<BTreeMap<String, Vec<ModPath>>, TreeError> {
    tree(files).map(|walked| walked.owners)
}

/// The walked tree, or the first declaration that cannot be resolved.
pub(crate) fn tree(files: &Files) -> Result<Tree, TreeError> {
    let mut walk = Walk {
        files,
        owners: BTreeMap::new(),
        modules: BTreeSet::new(),
        seen: BTreeSet::new(),
    };
    for (root, krate) in ROOTS {
        if !files.contains_key(root) {
            return Err(TreeError::Root(root.to_owned()));
        }
        walk.visit(
            root,
            &ModPath {
                krate,
                path: String::new(),
            },
            true,
        )?;
    }
    Ok(Tree {
        owners: walk
            .owners
            .into_iter()
            .map(|(file, owners)| (file, owners.into_iter().collect()))
            .collect(),
        modules: walk.modules,
    })
}

/// `a::b` joined with `c`, the root (`""`) absorbing the separator.
pub(crate) fn join_path(base: &str, name: &str) -> String {
    match (base.is_empty(), name.is_empty()) {
        (true, _) => name.to_owned(),
        (false, true) => base.to_owned(),
        (false, false) => format!("{base}::{name}"),
    }
}

struct Walk<'a> {
    files: &'a Files,
    owners: BTreeMap<String, BTreeSet<ModPath>>,
    modules: BTreeSet<ModPath>,
    seen: BTreeSet<(String, ModPath)>,
}

/// An inline `mod name { ... }` open at some brace depth.
struct Open {
    depth: usize,
    dir: String,
    name: String,
}

impl Walk<'_> {
    /// Walk `file` as module `at`. `dir_owner` is true for a crate root, a `mod.rs`, a
    /// file reached through `#[path]` and an `include!`d piece: its children resolve
    /// beside it rather than in a directory named after it.
    fn visit(&mut self, file: &str, at: &ModPath, dir_owner: bool) -> Result<(), TreeError> {
        if !self.seen.insert((file.to_owned(), at.clone())) {
            return Ok(());
        }
        self.owners
            .entry(file.to_owned())
            .or_default()
            .insert(at.clone());
        self.modules.insert(at.clone());
        let Some(text) = self.files.get(file) else {
            return Ok(());
        };
        let lexed = lex(text);
        let base = parent(file);
        let top = if dir_owner || file.ends_with("/mod.rs") {
            base.clone()
        } else {
            join_dir(&base, stem(file))
        };
        let mut open: Vec<Open> = Vec::new();
        let mut depth = 0usize;
        let mut attrs: Vec<(usize, usize)> = Vec::new();
        let mut index = 0;
        while index < lexed.tokens.len() {
            let token = &lexed.tokens[index];
            match token.kind {
                Kind::Punct(b'{') => depth += 1,
                Kind::Punct(b'}') => {
                    depth = depth.saturating_sub(1);
                    while open.last().is_some_and(|inner| inner.depth > depth) {
                        open.pop();
                    }
                }
                _ => {}
            }
            if let Some(end) = lexed.attribute_end(index) {
                attrs.push((index, end));
                index = end + 1;
                continue;
            }
            if lexed.is_ident(index, "pub") {
                index = lexed.skip_visibility(index);
                continue;
            }
            if lexed.is_ident(index, "mod")
                && let Some(name) = lexed.ident(index + 1)
            {
                let declared = Declared {
                    name: name.to_owned(),
                    line: lexed.line(token.start),
                    path_attr: lexed.path_attribute(&attrs),
                    inline: lexed.is_punct(index + 2, b'{'),
                };
                if declared.inline || lexed.is_punct(index + 2, b';') {
                    let inline: Vec<&str> = open.iter().map(|inner| inner.name.as_str()).collect();
                    let module = ModPath {
                        krate: at.krate,
                        path: join_path(&join_path(&at.path, &inline.join("::")), &declared.name),
                    };
                    let context = open.last().map(|inner| inner.dir.clone());
                    if declared.inline {
                        let dir = match &declared.path_attr {
                            Some(path) => join_dir(context.as_deref().unwrap_or(&base), path),
                            None => join_dir(context.as_deref().unwrap_or(&top), &declared.name),
                        };
                        self.modules.insert(module);
                        open.push(Open {
                            depth: depth + 1,
                            dir,
                            name: declared.name,
                        });
                    } else {
                        let (child, owner) =
                            self.child_file(file, &declared, context.as_deref(), &base, &top)?;
                        self.visit(&child, &module, owner)?;
                    }
                    attrs.clear();
                    index += 2;
                    continue;
                }
            }
            if let Some(included) = lexed.include_literal(index) {
                let inline: Vec<&str> = open.iter().map(|inner| inner.name.as_str()).collect();
                let module = ModPath {
                    krate: at.krate,
                    path: join_path(&at.path, &inline.join("::")),
                };
                let piece = normalize(&join_dir(&base, &included));
                if !self.files.contains_key(&piece) {
                    return Err(TreeError::Unresolved {
                        file: file.to_owned(),
                        line: lexed.line(token.start),
                        module: format!("include!(\"{included}\")"),
                    });
                }
                self.visit(&piece, &module, true)?;
            }
            attrs.clear();
            index += 1;
        }
        Ok(())
    }

    /// The file an out-of-line `mod name;` loads, and whether it owns its directory.
    fn child_file(
        &self,
        file: &str,
        declared: &Declared,
        context: Option<&str>,
        base: &str,
        top: &str,
    ) -> Result<(String, bool), TreeError> {
        if let Some(path) = &declared.path_attr {
            let child = normalize(&join_dir(context.unwrap_or(base), path));
            if self.files.contains_key(&child) {
                return Ok((child, true));
            }
        } else {
            let dir = context.unwrap_or(top);
            let flat = normalize(&join_dir(dir, &format!("{}.rs", declared.name)));
            if self.files.contains_key(&flat) {
                return Ok((flat, false));
            }
            let nested = normalize(&join_dir(dir, &format!("{}/mod.rs", declared.name)));
            if self.files.contains_key(&nested) {
                return Ok((nested, true));
            }
        }
        Err(TreeError::Unresolved {
            file: file.to_owned(),
            line: declared.line,
            module: declared.name.clone(),
        })
    }
}

struct Declared {
    name: String,
    line: usize,
    path_attr: Option<String>,
    inline: bool,
}

fn parent(file: &str) -> String {
    file.rsplit_once('/')
        .map_or_else(String::new, |(dir, _)| dir.to_owned())
}

fn stem(file: &str) -> &str {
    let name = file.rsplit_once('/').map_or(file, |(_, name)| name);
    name.strip_suffix(".rs").unwrap_or(name)
}

fn join_dir(dir: &str, rest: &str) -> String {
    if dir.is_empty() {
        rest.to_owned()
    } else {
        format!("{dir}/{rest}")
    }
}

/// `a/./b/../c` to `a/c`, purely on text. A `..` above the root stays, so it
/// matches no repository path.
pub(crate) fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|last| *last != "..") => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}

// ---------------------------------------------------------------------------
// The lexer: identifiers, punctuation and literals; comments dropped.
// ---------------------------------------------------------------------------

/// What one token is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Ident,
    /// A string literal; its index into [`Lexed::literals`].
    Literal(usize),
    /// A character literal, a number or a lifetime: never a name.
    Other,
    Punct(u8),
}

/// One token and its byte span in the source.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Token {
    pub(crate) kind: Kind,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// A file's tokens, its string literal values and where its lines start.
#[derive(Clone, Debug, Default)]
pub(crate) struct Lexed<'a> {
    pub(crate) text: &'a str,
    pub(crate) tokens: Vec<Token>,
    pub(crate) literals: Vec<String>,
    line_starts: Vec<usize>,
}

/// Tokenize Rust source. Comments vanish; string literals keep their raw contents.
pub(crate) fn lex(text: &str) -> Lexed<'_> {
    let bytes = text.as_bytes();
    let mut lexed = Lexed {
        text,
        line_starts: std::iter::once(0)
            .chain(
                bytes
                    .iter()
                    .enumerate()
                    .filter(|(_, byte)| **byte == b'\n')
                    .map(|(at, _)| at + 1),
            )
            .collect(),
        ..Lexed::default()
    };
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        if byte.is_ascii_whitespace() {
            at += 1;
        } else if bytes[at..].starts_with(b"//") {
            at = bytes[at..]
                .iter()
                .position(|b| *b == b'\n')
                .map_or(bytes.len(), |n| at + n);
        } else if bytes[at..].starts_with(b"/*") {
            at = block_comment_end(bytes, at);
        } else if let Some((end, value)) = string_literal(bytes, at) {
            let index = lexed.literals.len();
            lexed.literals.push(value);
            lexed.tokens.push(Token {
                kind: Kind::Literal(index),
                start: at,
                end,
            });
            at = end;
        } else if byte == b'\'' {
            let end = quote_end(bytes, at);
            lexed.tokens.push(Token {
                kind: Kind::Other,
                start: at,
                end,
            });
            at = end;
        } else if is_ident_start(byte) {
            let start = if bytes[at..].starts_with(b"r#") {
                at + 2
            } else {
                at
            };
            let end = start
                + bytes[start..]
                    .iter()
                    .take_while(|b| is_ident_byte(**b))
                    .count();
            lexed.tokens.push(Token {
                kind: Kind::Ident,
                start,
                end,
            });
            at = end;
        } else if byte.is_ascii_digit() {
            let end = at
                + bytes[at..]
                    .iter()
                    .take_while(|b| is_ident_byte(**b) || **b == b'.')
                    .count();
            lexed.tokens.push(Token {
                kind: Kind::Other,
                start: at,
                end,
            });
            at = end;
        } else {
            lexed.tokens.push(Token {
                kind: Kind::Punct(byte),
                start: at,
                end: at + 1,
            });
            at += 1;
        }
    }
    lexed
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn block_comment_end(bytes: &[u8], start: usize) -> usize {
    let mut depth = 0usize;
    let mut at = start;
    while at < bytes.len() {
        if bytes[at..].starts_with(b"/*") {
            depth += 1;
            at += 2;
        } else if bytes[at..].starts_with(b"*/") {
            depth -= 1;
            at += 2;
            if depth == 0 {
                return at;
            }
        } else {
            at += 1;
        }
    }
    bytes.len()
}

/// A string literal (`"…"`, `b"…"`, `c"…"`, `r#"…"#`, `br"…"`) starting at `at`:
/// its end and its raw contents.
fn string_literal(bytes: &[u8], at: usize) -> Option<(usize, String)> {
    let mut cursor = at;
    if matches!(bytes[cursor], b'b' | b'c') {
        cursor += 1;
    }
    let raw = bytes.get(cursor) == Some(&b'r');
    if raw {
        cursor += 1;
    }
    let hashes = bytes[cursor.min(bytes.len())..]
        .iter()
        .take_while(|b| **b == b'#')
        .count();
    if hashes > 0 && !raw {
        return None;
    }
    cursor += hashes;
    if bytes.get(cursor) != Some(&b'"') {
        return None;
    }
    let open = cursor + 1;
    let mut end = open;
    if raw {
        let mut close = vec![b'"'];
        close.extend(std::iter::repeat_n(b'#', hashes));
        while end < bytes.len() && !bytes[end..].starts_with(&close) {
            end += 1;
        }
        let value = String::from_utf8_lossy(&bytes[open..end.min(bytes.len())]).into_owned();
        return Some(((end + close.len()).min(bytes.len()), value));
    }
    while end < bytes.len() && bytes[end] != b'"' {
        end += if bytes[end] == b'\\' { 2 } else { 1 };
    }
    let value = String::from_utf8_lossy(&bytes[open..end.min(bytes.len())]).into_owned();
    Some(((end + 1).min(bytes.len()), value))
}

/// The end of a character literal or a lifetime that starts with `'` at `at`.
fn quote_end(bytes: &[u8], at: usize) -> usize {
    if bytes.get(at + 1) == Some(&b'\\') {
        let mut end = at + 2;
        while end < bytes.len() && bytes[end] != b'\'' && end < at + 12 {
            end += 1;
        }
        return (end + 1).min(bytes.len());
    }
    // One character (one to four UTF-8 bytes, by its lead byte) and a closing quote
    // is a char literal; `'a, 'b` is two lifetimes.
    let width = match bytes.get(at + 1) {
        Some(lead) if *lead >= 0xF0 => 4,
        Some(lead) if *lead >= 0xE0 => 3,
        Some(lead) if *lead >= 0xC0 => 2,
        _ => 1,
    };
    if bytes.get(at + 1 + width) == Some(&b'\'') {
        return at + 2 + width;
    }
    // A lifetime or a label: the quote and the name after it.
    at + 1
        + bytes[at + 1..]
            .iter()
            .take_while(|b| is_ident_byte(**b))
            .count()
}

impl Lexed<'_> {
    /// The 1-based line of a byte offset.
    pub(crate) fn line(&self, at: usize) -> usize {
        self.line_starts.partition_point(|start| *start <= at)
    }

    /// The identifier at `index`, if that token is one.
    pub(crate) fn ident(&self, index: usize) -> Option<&str> {
        let token = self.tokens.get(index)?;
        (token.kind == Kind::Ident).then(|| &self.text[token.start..token.end])
    }

    pub(crate) fn is_ident(&self, index: usize, name: &str) -> bool {
        self.ident(index) == Some(name)
    }

    pub(crate) fn is_punct(&self, index: usize, byte: u8) -> bool {
        self.tokens
            .get(index)
            .is_some_and(|token| token.kind == Kind::Punct(byte))
    }

    /// The string literal value at `index`, if that token is one.
    pub(crate) fn literal(&self, index: usize) -> Option<&str> {
        match self.tokens.get(index)?.kind {
            Kind::Literal(at) => self.literals.get(at).map(String::as_str),
            _ => None,
        }
    }

    /// Whether `index` and `index + 1` form `::`.
    pub(crate) fn is_path_sep(&self, index: usize) -> bool {
        self.is_punct(index, b':')
            && self.is_punct(index + 1, b':')
            && self.tokens[index].end == self.tokens[index + 1].start
    }

    /// The index of the bracket that closes the one at `open` (`(`, `[` or `{`).
    pub(crate) fn matching(&self, open: usize) -> Option<usize> {
        let (left, right) = match self.tokens.get(open)?.kind {
            Kind::Punct(b'(') => (b'(', b')'),
            Kind::Punct(b'[') => (b'[', b']'),
            Kind::Punct(b'{') => (b'{', b'}'),
            _ => return None,
        };
        let mut depth = 0usize;
        for index in open..self.tokens.len() {
            if self.is_punct(index, left) {
                depth += 1;
            } else if self.is_punct(index, right) {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
        }
        None
    }

    /// For an outer attribute `#[...]` starting at `index`, the index of its `]`.
    pub(crate) fn attribute_end(&self, index: usize) -> Option<usize> {
        if self.is_punct(index, b'#') && self.is_punct(index + 1, b'[') {
            return self.matching(index + 1);
        }
        if self.is_punct(index, b'#')
            && self.is_punct(index + 1, b'!')
            && self.is_punct(index + 2, b'[')
        {
            return self.matching(index + 2);
        }
        None
    }

    /// The last index of `pub`, `pub(crate)`, `pub(super)` or `pub(in path)` at `index`, plus one.
    pub(crate) fn skip_visibility(&self, index: usize) -> usize {
        if self.is_punct(index + 1, b'(')
            && let Some(close) = self.matching(index + 1)
        {
            return close + 1;
        }
        index + 1
    }

    /// The value of a `#[path = "..."]` among the attribute spans.
    fn path_attribute(&self, attrs: &[(usize, usize)]) -> Option<String> {
        attrs.iter().find_map(|(start, end)| {
            (*start..*end).find_map(|at| {
                (self.is_ident(at, "path") && self.is_punct(at + 1, b'='))
                    .then(|| self.literal(at + 2).map(str::to_owned))
                    .flatten()
            })
        })
    }

    /// The path of a literal `include!("x.rs")` at `index`.
    fn include_literal(&self, index: usize) -> Option<String> {
        if self.is_ident(index, "include")
            && self.is_punct(index + 1, b'!')
            && self.is_punct(index + 2, b'(')
        {
            let value = self.literal(index + 3)?;
            if self.is_punct(index + 4, b')')
                && std::path::Path::new(value)
                    .extension()
                    .is_some_and(|extension| extension == "rs")
            {
                return Some(value.to_owned());
            }
        }
        None
    }

    /// The inline `mod name { }` blocks of this file: (open token, close token, name).
    pub(crate) fn inline_modules(&self) -> Vec<(usize, usize, String)> {
        let mut found = Vec::new();
        for index in 0..self.tokens.len() {
            if self.is_ident(index, "mod")
                && let Some(name) = self.ident(index + 1)
                && self.is_punct(index + 2, b'{')
                && let Some(close) = self.matching(index + 2)
            {
                found.push((index + 2, close, name.to_owned()));
            }
        }
        found
    }

    /// The inline module names enclosing token `index`, outermost first.
    pub(crate) fn inline_path(spans: &[(usize, usize, String)], index: usize) -> String {
        spans
            .iter()
            .filter(|(open, close, _)| *open < index && index < *close)
            .map(|(_, _, name)| name.as_str())
            .collect::<Vec<_>>()
            .join("::")
    }
}
