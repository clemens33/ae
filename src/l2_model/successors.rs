//! The profile successor table: `[profile_successors]` in the GLOBAL config,
//! the human's declaration that a stopped seat recorded on one profile
//! resumes on another.
//!
//! ```text
//! [profile_successors]
//! sol61x = sol62x   # source profile = the profile it resumes on
//! ```
//!
//! The table only moves a STOPPED seat at its resume: a running seat keeps
//! the reseat route, a spawn keeps the profile it asks for, and a seat a
//! human flag names on the resume keeps that flag. A row moves a seat only
//! when both profiles prove to run on the same client and the same served
//! provider, so the conversation the seat records stays resumable.
//!
//! Every row is judged on its own, against the identity the caller launches
//! with; a row that fails is IGNORED with one note and leaves the other rows
//! usable:
//!
//! - its value is not one profile name, or its source is keyed twice;
//! - its source or its target is not a `[profiles]` row;
//! - it maps a profile to itself, it is a row of a cycle, or its target is
//!   the source of another row (one hop only: in `a = b`, `b = c` the row
//!   `a = b` is ignored and `b = c` is kept);
//! - the two profiles do not prove the same `[clients]` label, harness,
//!   assignment prefix and served provider (an unprovable provider fails
//!   closed).
//!
//! A key no profile can carry is inert, as in `[client_pairs]`.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::config::{self, IdentityConfig};

/// The config section the table is read from.
pub const SECTION: &str = "profile_successors";

/// The longest echo of a rejected value inside a note.
const ECHO: usize = 40;

/// What one source row says.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    /// The row moves its source to this target.
    Usable(String),
    /// The row is ignored; the reason is worded for the note.
    Ignored(String),
}

/// One source row, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    source: String,
    /// The written target, when the value was one profile name.
    target: Option<String>,
    verdict: Verdict,
}

/// The parsed and judged table.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Successors {
    rows: Vec<Row>,
    /// Why the whole section could not be read, if it could not.
    unreadable: Option<String>,
}

impl Successors {
    /// The profile a seat recorded on `profile` resumes on, if a usable row
    /// says so.
    #[must_use]
    pub fn target(&self, profile: &str) -> Option<&str> {
        self.rows.iter().find_map(|row| match &row.verdict {
            Verdict::Usable(target) if row.source == profile => Some(target.as_str()),
            _ => None,
        })
    }

    /// Whether the section says nothing at all: no row, nothing unreadable.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.unreadable.is_none()
    }

    /// The usable rows, as `(source, target)` in file order.
    #[must_use]
    pub fn usable(&self) -> Vec<(&str, &str)> {
        self.rows
            .iter()
            .filter_map(|row| match &row.verdict {
                Verdict::Usable(target) => Some((row.source.as_str(), target.as_str())),
                Verdict::Ignored(_) => None,
            })
            .collect()
    }

    /// One note per ignored row, in file order, plus one for a section that
    /// cannot be read. Each names the section, the source and the reason.
    #[must_use]
    pub fn notes(&self) -> Vec<String> {
        let mut notes: Vec<String> = Vec::new();
        if let Some(why) = &self.unreadable {
            notes.push(format!(
                "Note: [{SECTION}] cannot be read ({why}); no seat moves."
            ));
        }
        for row in &self.rows {
            if let Verdict::Ignored(why) = &row.verdict {
                notes.push(format!(
                    "Note: [{SECTION}] '{}' ignored — {why}.",
                    row.source
                ));
            }
        }
        notes
    }
}

/// A value echoed into a note: escaped and clipped, because it came out of a
/// hand-edited file.
fn echo(value: &str) -> String {
    let clipped: String = value.chars().take(ECHO).collect();
    format!("{clipped:?}")
}

/// Parse the section's SHAPE out of config text already read; `file` names
/// the text in a note. Rows are judged against no identity yet: every row
/// with a well-formed value reads usable until [`judge`] runs.
#[must_use]
pub fn parse_in(file: &Path, text: &str) -> Successors {
    let entries = match config::section_entries(file, text, SECTION) {
        Ok(entries) => entries,
        Err(why) => {
            return Successors {
                rows: Vec::new(),
                unreadable: Some(why),
            };
        }
    };
    let mut rows: Vec<Row> = Vec::new();
    for entry in entries {
        if !config::is_config_key(&entry.key) {
            continue;
        }
        let value = entry.value.as_deref().map(str::trim).unwrap_or_default();
        let (target, verdict) = if config::is_config_key(value) {
            (Some(value.to_owned()), Verdict::Usable(value.to_owned()))
        } else {
            (
                None,
                Verdict::Ignored(format!("value {} is not one profile name", echo(value))),
            )
        };
        match rows.iter_mut().find(|row| row.source == entry.key) {
            Some(existing) => {
                existing.target = None;
                existing.verdict = Verdict::Ignored("keyed twice".to_owned());
            }
            None => rows.push(Row {
                source: entry.key,
                target,
                verdict,
            }),
        }
    }
    Successors {
        rows,
        unreadable: None,
    }
}

/// Judge every well-formed row of `table` against `cfg`, the identity the
/// caller launches with. `home` resolves `$HOME`-rooted client homes.
#[must_use]
pub fn judge(mut table: Successors, cfg: &IdentityConfig, home: Option<&Path>) -> Successors {
    // The hop rules read the SHAPE of every row, never another row's verdict:
    // a row is not rescued because the row after it failed for its own reason.
    // Each source is unique (a twice-keyed one carries no target), so the
    // written edges form a graph of out-degree one, indexed once.
    let next: HashMap<String, String> = table
        .rows
        .iter()
        .filter_map(|row| {
            row.target
                .clone()
                .map(|target| (row.source.clone(), target))
        })
        .collect();
    let on_cycle = cycle_members(&next);
    let profiles: HashSet<&str> = cfg.profiles.iter().map(|(name, _)| name.as_str()).collect();
    let graph = Graph {
        next: &next,
        on_cycle: &on_cycle,
    };
    for row in &mut table.rows {
        let Verdict::Usable(target) = &row.verdict else {
            continue;
        };
        if let Some(why) = refusal(&row.source, target, &graph, &profiles, cfg, home) {
            row.verdict = Verdict::Ignored(why);
        }
    }
    table
}

/// The written edges and the sources that lie on a cycle of them.
struct Graph<'a> {
    next: &'a HashMap<String, String>,
    on_cycle: &'a HashSet<String>,
}

/// The longest path a cycle note spells before it elides the rest.
const NOTE_PATH: usize = 6;

/// Every profile that lies on a cycle of `next`, found in one pass: each walk
/// stops at the first profile an earlier walk already settled, so every
/// profile is stepped on at most twice.
fn cycle_members(next: &HashMap<String, String>) -> HashSet<String> {
    let mut settled: HashSet<&str> = HashSet::new();
    let mut members: HashSet<String> = HashSet::new();
    for start in next.keys() {
        if settled.contains(start.as_str()) {
            continue;
        }
        let mut path: Vec<&str> = Vec::new();
        let mut on_path: HashMap<&str, usize> = HashMap::new();
        let mut at = Some(start.as_str());
        while let Some(node) = at {
            if settled.contains(node) {
                break;
            }
            if let Some(&from) = on_path.get(node) {
                members.extend(path[from..].iter().map(|member| (*member).to_owned()));
                break;
            }
            on_path.insert(node, path.len());
            path.push(node);
            at = next.get(node).map(String::as_str);
        }
        settled.extend(path);
    }
    members
}

/// Parse and judge in one step.
#[must_use]
pub fn judged_in(file: &Path, text: &str, cfg: &IdentityConfig, home: Option<&Path>) -> Successors {
    judge(parse_in(file, text), cfg, home)
}

/// Why the row `source = target` is ignored, `None` when it is usable.
fn refusal(
    source: &str,
    target: &str,
    graph: &Graph<'_>,
    profiles: &HashSet<&str>,
    cfg: &IdentityConfig,
    home: Option<&Path>,
) -> Option<String> {
    if !profiles.contains(source) {
        return Some("source is not a profile".to_owned());
    }
    if !profiles.contains(target) {
        return Some(format!("target '{target}' is not a profile"));
    }
    if source == target {
        return Some("maps to itself".to_owned());
    }
    if let Some(why) = hop_refusal(source, target, graph) {
        return Some(why);
    }
    if !same_client_and_provider(cfg, source, target, home) {
        return Some(format!("'{target}' runs on another client or provider"));
    }
    None
}

/// The one-hop rule: a row of a cycle, or a row whose target moves on. The
/// cycle note spells the cycle from this source, elided past [`NOTE_PATH`].
fn hop_refusal(source: &str, target: &str, graph: &Graph<'_>) -> Option<String> {
    let first = graph.next.get(target)?;
    if !graph.on_cycle.contains(source) {
        return Some(format!(
            "chain longer than one hop ({source} -> {target} -> {first})"
        ));
    }
    let mut path: Vec<&str> = vec![source];
    let mut at = target;
    while at != source {
        if path.len() == NOTE_PATH {
            path.push("…");
            break;
        }
        path.push(at);
        at = graph.next.get(at).map_or(source, String::as_str);
    }
    path.push(source);
    Some(format!("cycle ({})", path.join(" -> ")))
}

/// Whether `source` and `target` prove the same client and served provider:
/// the same `[clients]` label, and resolved commands with the same harness,
/// the same assignment prefix and the same provable served provider.
fn same_client_and_provider(
    cfg: &IdentityConfig,
    source: &str,
    target: &str,
    home: Option<&Path>,
) -> bool {
    if cfg.profile_client_label(source) != cfg.profile_client_label(target) {
        return false;
    }
    let resolved = |profile: &str| {
        let command = cfg.command(profile, home).ok()??;
        let parsed = crate::launch_cmd::lex_simple_command(command.as_str()).ok()?;
        let provider = crate::tool::served_provider(command.as_str())?;
        Some((parsed.tool(), parsed.assign_span, provider))
    };
    match (resolved(source), resolved(target)) {
        (Some(from), Some(to)) => from == to,
        _ => false,
    }
}

/// The table the GLOBAL file at `global` declares, judged against `cfg`.
/// An absent file or one ae cannot read declares nothing: the table is empty,
/// and the caller's own read of the identity already refused or warned.
#[must_use]
pub fn read_global(global: &Path, cfg: &IdentityConfig, home: Option<&Path>) -> Successors {
    match config::read_selected(global) {
        Ok(text) => judged_in(global, &text, cfg, home),
        Err(_) => Successors::default(),
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::path::Path;

    use super::{Successors, judged_in, parse_in};

    const PROFILES: &str = "[clients]\nprimary = codex\nsecondary = codex config_home=/alt\n\
        anthropic = claude\n[profiles]\n\
        a = \"primary -m a\"\nb = \"primary -m b\"\nc = \"primary -m c\"\nd = \"primary -m d\"\n\
        other = \"secondary -m other\"\noss = \"primary --oss -m oss\"\n\
        cl = \"anthropic --model cl\"\nbare = \"codex -m bare\"\nopen = \"opencode -m x/y\"\n\
        open2 = \"opencode -m x/z\"\n";

    fn table(rows: &str) -> Successors {
        let text = format!("{PROFILES}[profile_successors]\n{rows}");
        let cfg = ae_cfg(&text);
        judged_in(Path::new("config"), &text, &cfg, Some(Path::new("/home/u")))
    }

    fn ae_cfg(text: &str) -> crate::config::IdentityConfig {
        crate::config::parse_identity(text).expect("fixture identity")
    }

    fn note_for(table: &Successors, source: &str) -> String {
        let marker = format!("'{source}' ignored");
        let found: Vec<String> = table
            .notes()
            .into_iter()
            .filter(|note| note.contains(&marker))
            .collect();
        assert_eq!(found.len(), 1, "{:?}", table.notes());
        found[0].clone()
    }

    #[test]
    fn a_usable_row_moves_its_source_and_nothing_else() {
        let t = table("a = b\n");
        assert_eq!(t.target("a"), Some("b"));
        assert_eq!(t.target("b"), None);
        assert!(t.notes().is_empty());
        assert_eq!(t.usable(), vec![("a", "b")]);
    }

    #[test]
    fn no_section_is_empty_and_says_nothing() {
        let text = PROFILES.to_owned();
        let cfg = ae_cfg(&text);
        let t = judged_in(Path::new("config"), &text, &cfg, None);
        assert!(t.is_empty());
        assert!(t.notes().is_empty());
        assert_eq!(t.target("a"), None);
    }

    #[test]
    fn shape_refusals_name_the_row() {
        let t = table("a = b c\n");
        assert!(note_for(&t, "a").contains("is not one profile name"));
        let t = table("a =\n");
        assert!(note_for(&t, "a").contains("is not one profile name"));
        let t = table("a = b@x\n");
        assert!(note_for(&t, "a").contains("is not one profile name"));
        let t = table("a = b\na = c\n");
        assert!(note_for(&t, "a").contains("keyed twice"));
        assert_eq!(t.target("a"), None);
    }

    #[test]
    fn a_key_no_profile_can_carry_is_inert() {
        let t = table("bad key = b\n");
        assert!(t.notes().is_empty());
        assert!(t.usable().is_empty());
    }

    #[test]
    fn source_and_target_must_be_profiles() {
        let t = table("gone = b\n");
        assert!(note_for(&t, "gone").contains("source is not a profile"));
        let t = table("a = missing\n");
        assert!(note_for(&t, "a").contains("target 'missing' is not a profile"));
    }

    #[test]
    fn a_self_map_and_both_rows_of_a_cycle_are_ignored() {
        let t = table("a = a\n");
        assert!(note_for(&t, "a").contains("maps to itself"));
        let t = table("a = b\nb = a\n");
        assert!(note_for(&t, "a").contains("cycle (a -> b -> a)"));
        assert!(note_for(&t, "b").contains("cycle (b -> a -> b)"));
        let t = table("a = b\nb = c\nc = a\n");
        for source in ["a", "b", "c"] {
            assert!(note_for(&t, source).contains("cycle"), "{source}");
        }
    }

    #[test]
    fn only_the_upstream_row_of_a_chain_is_ignored() {
        let t = table("a = b\nb = c\nd = c\n");
        assert!(note_for(&t, "a").contains("chain longer than one hop (a -> b -> c)"));
        assert_eq!(t.target("b"), Some("c"));
        assert_eq!(t.target("d"), Some("c"));
        assert_eq!(t.notes().len(), 1);
    }

    #[test]
    fn a_long_cycle_note_is_bounded() {
        let names: Vec<String> = (0..40).map(|at| format!("p{at}")).collect();
        let mut text = String::from("[clients]\nprimary = codex\n[profiles]\n");
        for name in &names {
            let _ = writeln!(text, "{name} = \"primary -m {name}\"");
        }
        text.push_str("[profile_successors]\n");
        for (at, name) in names.iter().enumerate() {
            let _ = writeln!(text, "{name} = {}", names[(at + 1) % names.len()]);
        }
        let cfg = ae_cfg(&text);
        let t = judged_in(Path::new("config"), &text, &cfg, None);
        assert_eq!(t.notes().len(), names.len());
        let note = note_for(&t, "p0");
        assert!(
            note.contains("cycle (p0 -> p1 -> p2 -> p3 -> p4 -> p5 -> … -> p0)"),
            "{note}"
        );
    }

    #[test]
    fn a_chain_into_a_cycle_ends_and_is_named_a_chain() {
        let t = table("d = a\na = b\nb = a\n");
        assert!(note_for(&t, "d").contains("chain longer than one hop (d -> a -> b)"));
        assert!(note_for(&t, "a").contains("cycle"));
    }

    #[test]
    fn a_voided_row_is_no_hop() {
        // `b` is keyed twice, so its row is voided and moves nothing: the hop
        // rule reads only well-formed rows, so `a = b` stays usable.
        let t = table("a = b\nb = c\nb = d\n");
        assert!(note_for(&t, "b").contains("keyed twice"));
        assert_eq!(t.target("a"), Some("b"));
        assert_eq!(t.target("b"), None);
    }

    #[test]
    fn another_client_harness_or_provider_is_refused() {
        for (row, source) in [
            ("a = other\n", "a"),
            ("a = cl\n", "a"),
            ("a = oss\n", "a"),
            ("a = bare\n", "a"),
            ("open = open2\n", "open"),
        ] {
            let t = table(row);
            assert!(
                note_for(&t, source).contains("runs on another client or provider"),
                "{row}"
            );
        }
    }

    #[test]
    fn an_unreadable_section_moves_nothing_and_says_so() {
        let text = "[profile_successors]\ninstructions = \"\"\"\n";
        let t = parse_in(Path::new("config"), text);
        assert_eq!(t.target("a"), None);
        assert!(!t.is_empty());
        assert_eq!(t.notes().len(), 1);
    }
}
