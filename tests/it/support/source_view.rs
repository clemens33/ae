//! One honest source view for the text guards.
//!
//! A text guard asks "does production code spell X". It is only as honest as
//! what it was shown, so this module owns what a guard may be shown: every
//! production byte of every `src/` file, or a failure naming the file and line
//! the view could not classify. Unknown never reads as test code.
//!
//! The view of one file ([`classify`]) keeps the source's length and newlines:
//! comments and every `#[cfg(test)]` item are replaced by spaces, so an offset
//! or a line number in the view is the source's own. String and character
//! literals stay verbatim, because the live-file spelling guards read them.
//! The view of the tree ([`product_of`]) follows `mod` registrations from the
//! crate roots by rustc's own file rule, so a file is test-only because it is
//! registered by test code alone, never because of what it is called.
//!
//! SYNTAX. `syn` (a dev-dependency) lexes and parses each file; it drops comments,
//! so a comment is what lies between two tokens. A `#[cfg(test)]` is cut only on
//! a node [`Walk`] lists: items and their members, variants, fields, arms, an `if`
//! or block, a macro statement. A gate anywhere else (a macro body included, which
//! `syn` leaves opaque), an inner one, and a file `syn` cannot read are a
//! [`Failure`]: adding a node is a review, never a fallback.
//!
//! Attributes: `cfg(test)` gates; `cfg(not(test))` does not; `cfg_attr` is read
//! only when every attribute it adds is `allow`, `expect`, `warn`, `deny` or
//! `forbid`; any other `cfg(..)` that mentions `test` is refused.

#![allow(
    clippy::disallowed_methods,
    reason = "this module is the view's one door to the filesystem: it reads the crate's own \
              source, and the capability boundary is about what PRODUCT code may reach"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::ops::Range;
use std::path::Path;
use std::sync::OnceLock;

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{AttrStyle, Attribute, Expr, ExprLit, ItemMod, Lit, Meta, MetaNameValue};

/// A file the view could not read or classify, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    /// The file, as the view names it (root-relative, `/`-separated).
    pub(crate) file: String,
    /// The source line it could not classify; `0` only for a failed read.
    pub(crate) line: usize,
    /// Why.
    pub(crate) what: String,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.what)
    }
}

fn fail(file: &str, line: usize, what: impl Into<String>) -> Failure {
    Failure {
        file: file.to_owned(),
        line,
        what: what.into(),
    }
}

/// One inline module a registration sits inside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Nest {
    pub(crate) name: String,
    pub(crate) path_attr: Option<String>,
}

/// One `mod NAME;` of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Registration {
    pub(crate) name: String,
    /// The line of its `mod` keyword.
    pub(crate) line: usize,
    /// The `#[path = ".."]` value, as written.
    pub(crate) path_attr: Option<String>,
    /// Gated by `#[cfg(test)]`, or inside something that is.
    pub(crate) gated: bool,
    /// The inline modules around it, outermost first.
    pub(crate) nesting: Vec<Nest>,
}

/// One file, classified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Classified {
    /// The source with comments and `#[cfg(test)]` items blanked.
    pub(crate) production: String,
    pub(crate) registrations: Vec<Registration>,
}

// ---- the lexer ---------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Region {
    Comment,
    Literal,
}

struct Span {
    start: usize,
    end: usize,
    region: Region,
}

fn line_at(text: &str, at: usize) -> usize {
    1 + text.bytes().take(at).filter(|byte| *byte == b'\n').count()
}

/// A file that does not lex or parse proves no `#[cfg(test)]` gates a known
/// item: blame the last one before the error (at the end, when the error has
/// no place), else the error's own line.
fn unparsed(file: &str, text: &str, span: proc_macro2::Span, why: &str) -> Failure {
    let at = Some(span.byte_range().start).filter(|at| *at > 0);
    let gate = text
        .get(..at.unwrap_or(text.len()))
        .and_then(|head| head.rfind("cfg(test)"));
    let own = span.start().line;
    let line = gate.map_or(own, |gate| line_at(text, gate));
    fail(
        file,
        line,
        format!("the file does not lex or parse (line {own}): {why}"),
    )
}

/// One pass over a file's tokens. `syn` drops comments, so a comment is what
/// sits between two tokens; a literal is a string or character token; every
/// attribute is kept, macro bodies included, with the offset of its `#`.
#[derive(Default)]
struct Lexed {
    spans: Vec<Span>,
    attrs: Vec<(usize, bool, TokenStream)>,
    /// The end of the last token.
    cursor: usize,
}

impl Lexed {
    fn token(&mut self, text: &str, range: Range<usize>, literal: bool) {
        let gap = text.get(self.cursor..range.start).unwrap_or_default();
        let start = self.cursor + gap.len() - gap.trim_start().len();
        let end = self.cursor + gap.trim_end().len();
        if start < end {
            self.spans.push(Span {
                start,
                end,
                region: Region::Comment,
            });
        }
        if literal {
            // A `b` or `c` prefix stays outside the literal, a raw `r` inside.
            let prefix = matches!(text.as_bytes().get(range.start), Some(b'b' | b'c'));
            let start = range.start + usize::from(prefix);
            self.spans.push(Span {
                start,
                end: range.end,
                region: Region::Literal,
            });
        }
        self.cursor = range.end;
    }

    fn walk(&mut self, text: &str, tokens: TokenStream) {
        let trees: Vec<TokenTree> = tokens.into_iter().collect();
        for (at, tree) in trees.iter().enumerate() {
            let range = tree.span().byte_range();
            // A doc comment is tokens that all span the comment: it stays a gap.
            if range.len() > 1 && text.get(range.clone()).is_some_and(|s| s.starts_with('/')) {
                continue;
            }
            match tree {
                TokenTree::Group(group) => {
                    self.token(text, group.span_open().byte_range(), false);
                    self.walk(text, group.stream());
                    self.token(text, group.span_close().byte_range(), false);
                }
                TokenTree::Literal(_) => {
                    let digit = text
                        .as_bytes()
                        .get(range.start)
                        .is_some_and(u8::is_ascii_digit);
                    self.token(text, range, !digit);
                }
                TokenTree::Punct(pound) if pound.as_char() == '#' => {
                    let inner = matches!(trees.get(at + 1), Some(TokenTree::Punct(bang)) if bang.as_char() == '!');
                    if let Some(TokenTree::Group(body)) = trees.get(at + 1 + usize::from(inner))
                        && body.delimiter() == Delimiter::Bracket
                    {
                        self.attrs.push((range.start, inner, body.stream()));
                    }
                    self.token(text, range, false);
                }
                _ => self.token(text, range, false),
            }
        }
    }
}

/// Lex `text`: its tokens, and what [`Lexed`] found in them.
fn lexed(file: &str, text: &str) -> Result<(TokenStream, Lexed), Failure> {
    let tokens = text
        .parse::<TokenStream>()
        .map_err(|err| unparsed(file, text, err.span(), &err.to_string()))?;
    let mut lexed = Lexed::default();
    lexed.walk(text, tokens.clone());
    lexed.token(text, text.len()..text.len(), false);
    Ok((tokens, lexed))
}

/// Replace `start..end` with spaces, keeping newlines, so offsets and lines hold.
fn blank_range(bytes: &mut [u8], start: usize, end: usize) {
    for byte in bytes.iter_mut().take(end).skip(start) {
        if *byte != b'\n' {
            *byte = b' ';
        }
    }
}

fn blanked(
    file: &str,
    text: &str,
    spans: &[Span],
    blank: impl Fn(Region) -> bool,
) -> Result<String, Failure> {
    let mut bytes = text.as_bytes().to_vec();
    for span in spans.iter().filter(|span| blank(span.region)) {
        blank_range(&mut bytes, span.start, span.end);
    }
    into_text(file, bytes)
}

fn into_text(file: &str, bytes: Vec<u8>) -> Result<String, Failure> {
    String::from_utf8(bytes).map_err(|_| fail(file, 0, "a blanked range split a character"))
}

/// `text` with every comment (line, doc, nested block) blanked to spaces.
pub(crate) fn without_comments(file: &str, text: &str) -> Result<String, Failure> {
    let (_, lexed) = lexed(file, text)?;
    blanked(file, text, &lexed.spans, |region| region == Region::Comment)
}

/// `text` with every string and character literal blanked to spaces, quotes
/// included, one space per byte.
pub(crate) fn blank_literals(file: &str, text: &str) -> Result<String, Failure> {
    let (_, lexed) = lexed(file, text)?;
    blanked(file, text, &lexed.spans, |region| region == Region::Literal)
}

// ---- the classifier ----------------------------------------------------------

fn mentions_test(tokens: TokenStream) -> bool {
    tokens.into_iter().any(|tree| match tree {
        TokenTree::Ident(word) => word == "test",
        TokenTree::Group(group) => mentions_test(group.stream()),
        _ => false,
    })
}

/// Whether the attribute `body` gates (`Ok(true)`); `cfg(not(test))` does not;
/// `cfg_attr` is read only when every attribute it adds is a lint; any other
/// `cfg(..)` that mentions `test` is refused.
fn gates(body: &TokenStream) -> Result<bool, String> {
    let norm: String = body.to_string().split_whitespace().collect();
    if norm.starts_with("cfg_attr(")
        && let Some(TokenTree::Group(args)) = body.clone().into_iter().nth(1)
    {
        let (mut part, mut head) = (0_usize, true);
        for tree in args.stream() {
            if matches!(&tree, TokenTree::Punct(comma) if comma.as_char() == ',') {
                (part, head) = (part + 1, true);
            } else {
                let lint = matches!(&tree, TokenTree::Ident(name)
                    if ["allow", "expect", "warn", "deny", "forbid"].iter().any(|l| name == l));
                if part > 0 && head && !lint {
                    return Err(format!(
                        "a cfg_attr that may change cfg or path (`{tree}`); only \
                         allow, expect, warn, deny and forbid are read"
                    ));
                }
                head = false;
            }
        }
    } else if norm.starts_with("cfg(")
        && !matches!(norm.as_str(), "cfg(test)" | "cfg(not(test))")
        && mentions_test(body.clone())
    {
        return Err(format!("an unrecognised cfg mentioning test: {norm}"));
    }
    Ok(norm == "cfg(test)")
}

fn is_gate(attr: &Attribute) -> bool {
    matches!(&attr.meta, Meta::List(list)
        if matches!(attr.style, AttrStyle::Outer) && list.path.is_ident("cfg") && list.tokens.to_string() == "test")
}

/// The walk over one file's syntax tree: the registrations, and the span of
/// every outermost `#[cfg(test)]` node. Only the nodes a gate may sit on are
/// listed; a gate anywhere else is a failure ([`classify`]).
struct Walk<'a> {
    file: &'a str,
    text: &'a str,
    /// `text` with comments and literals blanked.
    structural: &'a [u8],
    gated: bool,
    nest: Vec<Nest>,
    cuts: Vec<(usize, usize)>,
    regs: Vec<Registration>,
    failure: Option<Failure>,
}

impl Walk<'_> {
    /// The node's bytes, with the `,` that follows a variant, field or arm.
    fn range(&self, node: &impl Spanned) -> (usize, usize) {
        let span = node.span().byte_range();
        let rest = self.structural.get(span.end..).unwrap_or_default();
        let comma = rest.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b',');
        let skipped = rest
            .iter()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
        (span.start, span.end + if comma { skipped + 1 } else { 0 })
    }

    /// A node carrying `attrs`: cut when it is gated, walked as gated inside.
    fn scoped<N: Spanned>(&mut self, attrs: &[Attribute], node: &N, walk: impl FnOnce(&mut Self)) {
        let gate = attrs.iter().any(is_gate);
        if gate && !self.gated {
            let cut = self.range(node);
            self.cuts.push(cut);
        }
        let outer = self.gated;
        self.gated |= gate;
        walk(self);
        self.gated = outer;
    }

    /// The `#[path = ".."]` of a module, which must be one plain relative literal.
    fn path_attr(&mut self, attrs: &[Attribute]) -> Option<String> {
        let attr = attrs.iter().rfind(|attr| attr.path().is_ident("path"))?;
        let value = match &attr.meta {
            Meta::NameValue(MetaNameValue {
                value:
                    Expr::Lit(ExprLit {
                        lit: Lit::Str(lit), ..
                    }),
                ..
            }) if !lit.token().to_string().contains('\\') && !lit.value().starts_with('/') => {
                Some(lit.value())
            }
            _ => None,
        };
        if value.is_none() && self.failure.is_none() {
            let line = line_at(self.text, attr.pound_token.span.byte_range().start);
            self.failure = Some(fail(
                self.file,
                line,
                "a #[path] that is not one plain relative string literal",
            ));
        }
        value
    }
}

/// One `fn` per node kind a gate may sit on: its items, their members, enum
/// variants, fields (declared or initialised), match arms, an `if` statement
/// and a macro statement.
macro_rules! gate_sites {
    ($($visit:ident($node:ident)),* $(,)?) => {$(
        fn $visit(&mut self, node: &'ast syn::$node) {
            self.scoped(&node.attrs, node, |walk| visit::$visit(walk, node));
        }
    )*};
}

impl<'ast> Visit<'ast> for Walk<'_> {
    gate_sites! {
        visit_item_const(ItemConst), visit_item_enum(ItemEnum), visit_item_extern_crate(ItemExternCrate),
        visit_item_fn(ItemFn), visit_item_foreign_mod(ItemForeignMod), visit_item_impl(ItemImpl),
        visit_item_macro(ItemMacro), visit_item_static(ItemStatic), visit_item_struct(ItemStruct),
        visit_item_trait(ItemTrait), visit_item_trait_alias(ItemTraitAlias), visit_item_type(ItemType),
        visit_item_union(ItemUnion), visit_item_use(ItemUse),
        visit_impl_item_const(ImplItemConst), visit_impl_item_fn(ImplItemFn),
        visit_impl_item_macro(ImplItemMacro), visit_impl_item_type(ImplItemType),
        visit_trait_item_const(TraitItemConst), visit_trait_item_fn(TraitItemFn),
        visit_trait_item_macro(TraitItemMacro), visit_trait_item_type(TraitItemType),
        visit_foreign_item_fn(ForeignItemFn), visit_foreign_item_static(ForeignItemStatic),
        visit_foreign_item_type(ForeignItemType), visit_foreign_item_macro(ForeignItemMacro),
        visit_variant(Variant), visit_field(Field), visit_field_value(FieldValue), visit_arm(Arm),
        visit_expr_block(ExprBlock), visit_expr_if(ExprIf), visit_stmt_macro(StmtMacro),
    }

    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        self.scoped(&node.attrs, node, |walk| {
            let path_attr = walk.path_attr(&node.attrs);
            let name = node.ident.to_string();
            if node.content.is_some() {
                walk.nest.push(Nest { name, path_attr });
                visit::visit_item_mod(walk, node);
                walk.nest.pop();
            } else {
                walk.regs.push(Registration {
                    name,
                    line: line_at(walk.text, node.mod_token.span.byte_range().start),
                    path_attr,
                    gated: walk.gated,
                    nesting: walk.nest.clone(),
                });
            }
        });
    }
}

/// Classify one file: its production text and its module registrations, or the
/// file and line the view could not read. `file` only names diagnostics.
pub(crate) fn classify(file: &str, text: &str) -> Result<Classified, Failure> {
    let (tokens, lexed) = lexed(file, text)?;
    let tree = syn::parse2::<syn::File>(tokens)
        .map_err(|err| unparsed(file, text, err.span(), &err.to_string()))?;
    let structural = blanked(file, text, &lexed.spans, |_| true)?;
    let mut gated_at = Vec::new();
    for (at, inner, body) in lexed.attrs {
        match gates(&body) {
            Err(why) => return Err(fail(file, line_at(text, at), why)),
            Ok(true) if inner => {
                let why = "an inner #![cfg(test)] gates the whole file";
                return Err(fail(file, line_at(text, at), why));
            }
            Ok(true) => gated_at.push(at),
            Ok(false) => {}
        }
    }
    let mut walk = Walk {
        file,
        text,
        structural: structural.as_bytes(),
        gated: false,
        nest: Vec::new(),
        cuts: Vec::new(),
        regs: Vec::new(),
        failure: None,
    };
    walk.visit_file(&tree);
    if let Some(failure) = walk.failure {
        return Err(failure);
    }
    // A gate no listed node owns is a gate this view cannot place.
    if let Some(at) = gated_at.into_iter().find(|at| {
        !walk
            .cuts
            .iter()
            .any(|(start, end)| (start..end).contains(&at))
    }) {
        let why = "a #[cfg(test)] on a node this view does not read";
        return Err(fail(file, line_at(text, at), why));
    }
    let mut bytes = text.as_bytes().to_vec();
    for span in lexed
        .spans
        .iter()
        .filter(|span| span.region == Region::Comment)
    {
        blank_range(&mut bytes, span.start, span.end);
    }
    for (start, end) in walk.cuts {
        blank_range(&mut bytes, start, end);
    }
    Ok(Classified {
        production: into_text(file, bytes)?,
        registrations: walk.regs,
    })
}

// ---- file identity -----------------------------------------------------------

/// Where a registration points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// A loaded `src/` file.
    Src(String),
    /// A path outside `src/` (a test file registered with `#[path]`).
    Outside(String),
}

/// `dir/rel` with `.` and `..` resolved lexically.
fn join(dir: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in dir.split('/').chain(rel.split('/')) {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|last| *last != "..") => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// The file `reg` (a registration in `file`) names, by rustc's rule: a module
/// file of a `lib.rs`, `main.rs` or `mod.rs` sits in its directory, any other
/// file `a/b.rs` owns `a/b/`, an inline module adds its name (or its
/// `#[path]`, which also drops the file's own stem), and `#[path]` on a file
/// module is relative to the directory of the registering file.
pub(crate) fn target_of(file: &str, reg: &Registration, known: &[&str]) -> Result<Target, Failure> {
    let (parent, leaf) = file.rsplit_once('/').unwrap_or(("", file));
    let mut dir = parent.to_owned();
    let mut relative = (!matches!(leaf, "lib.rs" | "main.rs" | "mod.rs"))
        .then(|| leaf.trim_end_matches(".rs").to_owned());
    for inline in &reg.nesting {
        if let Some(path) = &inline.path_attr {
            dir = join(&dir, path);
            relative = None;
        } else {
            if let Some(stem) = relative.take() {
                dir = join(&dir, &stem);
            }
            dir = join(&dir, &inline.name);
        }
    }
    let unresolved = |why: String| fail(file, reg.line, why);
    if let Some(path) = &reg.path_attr {
        let target = join(&dir, path);
        if !target.starts_with("src/") {
            return Ok(Target::Outside(target));
        }
        return if known.contains(&target.as_str()) {
            Ok(Target::Src(target))
        } else {
            Err(unresolved(format!(
                "`mod {}` #[path] names {target}, which is not a loaded source file",
                reg.name
            )))
        };
    }
    if let Some(stem) = relative {
        dir = join(&dir, &stem);
    }
    let flat = join(&dir, &format!("{}.rs", reg.name));
    let nested = join(&dir, &format!("{}/mod.rs", reg.name));
    match (
        known.contains(&flat.as_str()),
        known.contains(&nested.as_str()),
    ) {
        (true, false) => Ok(Target::Src(flat)),
        (false, true) => Ok(Target::Src(nested)),
        (true, true) => Err(unresolved(format!(
            "`mod {}` is ambiguous: both {flat} and {nested} exist",
            reg.name
        ))),
        (false, false) => Err(unresolved(format!(
            "`mod {}` names neither {flat} nor {nested}",
            reg.name
        ))),
    }
}

/// A source file's production text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProductFile {
    pub(crate) path: String,
    pub(crate) text: String,
}

/// What the text guards may be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Product {
    /// Every file reachable from a crate root through registrations that are
    /// not gated, sorted by path.
    pub(crate) files: Vec<ProductFile>,
    /// Every other file, each registered by test code alone, sorted.
    pub(crate) test_only: Vec<String>,
}

/// The crate roots a registration chain starts from.
const ROOTS: [&str; 2] = ["src/lib.rs", "src/main.rs"];

/// Classify every file and decide which are production and which test-only.
/// `files` are `(root-relative path, text)`. A file registered by nothing, a
/// registration that names no file and an ungated registration that leaves
/// `src/` are failures: a file the view cannot place never silently drops out.
pub(crate) fn product_of(files: &[(String, String)]) -> Result<Product, Failure> {
    let known: Vec<&str> = files.iter().map(|(path, _)| path.as_str()).collect();
    let mut classified = BTreeMap::new();
    for (path, text) in files {
        classified.insert(path.as_str(), classify(path, text)?);
    }
    let regs = |file: &str| -> Vec<Registration> {
        classified
            .get(file)
            .map(|class| class.registrations.clone())
            .unwrap_or_default()
    };
    let roots: Vec<&str> = ROOTS
        .into_iter()
        .filter(|root| known.contains(root))
        .collect();
    if roots.is_empty() {
        return Err(fail(ROOTS[0], 1, "no crate root among the loaded files"));
    }

    let mut production: BTreeSet<String> = roots.iter().map(|root| (*root).to_owned()).collect();
    let mut queue: Vec<String> = production.iter().cloned().collect();
    while let Some(file) = queue.pop() {
        for reg in regs(&file).iter().filter(|reg| !reg.gated) {
            match target_of(&file, reg, &known)? {
                Target::Src(target) => {
                    if production.insert(target.clone()) {
                        queue.push(target);
                    }
                }
                Target::Outside(target) => {
                    return Err(fail(
                        &file,
                        reg.line,
                        format!(
                            "an ungated `mod {}` registers {target}, outside src/",
                            reg.name
                        ),
                    ));
                }
            }
        }
    }

    // A production file's GATED registrations seed the test-only set; from a
    // test-only file every registration, gated or not, stays test-only.
    let mut test_only = BTreeSet::new();
    let mut frontier: Vec<(String, bool)> = production.iter().map(|f| (f.clone(), true)).collect();
    while let Some((file, gated_only)) = frontier.pop() {
        for reg in regs(&file).iter().filter(|reg| reg.gated || !gated_only) {
            if let Target::Src(target) = target_of(&file, reg, &known)?
                && !production.contains(&target)
                && test_only.insert(target.clone())
            {
                frontier.push((target, false));
            }
        }
    }

    for path in &known {
        if !production.contains(*path) && !test_only.contains(*path) {
            return Err(fail(
                path,
                1,
                "registered by no module reachable from src/lib.rs or src/main.rs, \
                 and by no test-only registration",
            ));
        }
    }
    Ok(Product {
        files: production
            .into_iter()
            .map(|path| ProductFile {
                text: classified
                    .get(path.as_str())
                    .map(|class| class.production.clone())
                    .unwrap_or_default(),
                path,
            })
            .collect(),
        test_only: test_only.into_iter().collect(),
    })
}

// ---- the filesystem door -----------------------------------------------------

fn walk(root: &Path, dir: &str, out: &mut Vec<(String, String)>) -> Result<(), Failure> {
    let entries = fs::read_dir(root.join(dir))
        .map_err(|err| fail(dir, 0, format!("cannot read the directory: {err}")))?;
    for entry in entries {
        let entry = entry.map_err(|err| fail(dir, 0, format!("cannot read an entry: {err}")))?;
        let name = entry.file_name().into_string().map_err(|name| {
            fail(
                dir,
                0,
                format!("a file name that is not UTF-8: {}", name.display()),
            )
        })?;
        let path = format!("{dir}/{name}");
        let kind = entry
            .file_type()
            .map_err(|err| fail(&path, 0, format!("cannot read the file type: {err}")))?;
        if kind.is_symlink() {
            return Err(fail(
                &path,
                0,
                "a symbolic link in a scanned tree is not followed",
            ));
        }
        if kind.is_dir() {
            walk(root, &path, out)?;
        } else if Path::new(&name).extension().is_some_and(|ext| ext == "rs") {
            let text = fs::read_to_string(root.join(&path))
                .map_err(|err| fail(&path, 0, format!("cannot read the file: {err}")))?;
            out.push((path, text));
        }
    }
    Ok(())
}

/// Every `.rs` file under each of `dirs` (relative to `root`), as
/// `(root-relative path, text)`, sorted. A directory, an entry or a file that
/// cannot be read, a name or text that is not UTF-8 and a symbolic link are
/// failures that name the path: nothing is skipped.
pub(crate) fn load(root: &Path, dirs: &[&str]) -> Result<Vec<(String, String)>, Failure> {
    let mut out = Vec::new();
    for dir in dirs {
        walk(root, dir, &mut out)?;
    }
    out.sort();
    Ok(out)
}

/// Every `.rs` file of `src/` and `tests/` of this crate, read once. Panics
/// with the failure's `file:line:` when any of it cannot be read.
pub(crate) fn tree() -> &'static [(String, String)] {
    static TREE: OnceLock<Result<Vec<(String, String)>, Failure>> = OnceLock::new();
    match TREE.get_or_init(|| load(Path::new(env!("CARGO_MANIFEST_DIR")), &["src", "tests"])) {
        Ok(files) => files,
        Err(failure) => panic!("{failure}"),
    }
}

/// The product view of this crate's `src/`, built once. Panics with the
/// failure's `file:line:` when `src/` cannot be read or classified.
pub(crate) fn product() -> &'static Product {
    static PRODUCT: OnceLock<Result<Product, Failure>> = OnceLock::new();
    match PRODUCT.get_or_init(|| {
        let src: Vec<(String, String)> = tree()
            .iter()
            .filter(|(path, _)| path.starts_with("src/"))
            .cloned()
            .collect();
        product_of(&src)
    }) {
        Ok(product) => product,
        Err(failure) => panic!("{failure}"),
    }
}
