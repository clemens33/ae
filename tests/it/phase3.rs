//! Phase 3 end to end: one classified snapshot in, the surfaces an operator or
//! a consumer sees out.
//!
//! Gate: `docs/migration/p1-phase3-gate.md`, blob
//! `8cccbe44787d4ea6007ad9cf9d1cc83a3d03936c` — fifteen criteria. Each test
//! names the one it answers.
//!
//! # The reference snapshot
//!
//! Four candidates per status group, named to be hostile to every order that is
//! NOT the ratified one: `AlphaR`, `ZetaR`, `alpha10R`, `alpha9R` (and the `U`
//! and `S` suffixes for unknown and stopped). Those names separate C-byte order
//! from case-folded order (`AlphaR` before `ZetaR` before `alpha*` only if case
//! is significant), from natural-number order (`alpha10R` before `alpha9R` only
//! if digits are compared as text), and from creation order (they are planted
//! backwards).
//!
//! Per group the four rows carry independent attention/activity facts:
//! attention-only, activity-only, both, neither.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;

use ae::attention::Reason;
use ae::digest::{AgentEntry, SessionEntry, Status};
use ae::filters::{DEFAULT_ACTIVE_WINDOW_SECS, ListArgs};
use ae::inventory::{FailedSource, Roots, durable_records};
use ae::json;
use ae::listing::{World, diagnostic, render};
use ae::meta::Meta;
use ae::session::{DEFAULT_UNANSWERED_SECS, SessionRead, SessionRuntime, entry_for};
use ae::time::Timestamp;
use std::path::{Path, PathBuf};

use super::parity::Invocation;
use super::parity::capture::ExitOutcome;
use super::parity::capture::raw;

const NOW: Timestamp = Timestamp::from_epoch(1_780_000_000);

/// The four name stems, in the order the gate pre-registers as correct.
const C_ORDER: [&str; 4] = ["Alpha", "Zeta", "alpha10", "alpha9"];

/// The gate's four independent fact pairs, one per row in a group.
const FACTS: [(bool, bool); 4] = [(true, false), (false, true), (true, true), (false, false)];

fn suffix(status: Status) -> &'static str {
    match status {
        Status::Running => "R",
        Status::Unknown => "U",
        Status::Stopped => "S",
    }
}

/// One planted row, and everything the manifest records about it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Planted {
    name: String,
    status: Status,
    attention: bool,
    active: bool,
    degraded: bool,
    creation_position: usize,
}

/// The reference snapshot, as a manifest plus the entries themselves.
///
/// The manifest exists so every later assertion can be read against what was
/// PLANTED rather than against what came out — a fixture that describes itself
/// only through its output cannot tell you it planted the wrong thing.
struct Reference {
    manifest: Vec<Planted>,
    entries: Vec<SessionEntry>,
}

impl Reference {
    /// Build the reference snapshot, supplied in `order`.
    fn new(order: Supply) -> Self {
        let mut manifest = Vec::new();
        let mut creation = 0;
        // Creation order is deliberately OPPOSED to C order: reversed stems,
        // and stopped before unknown before running.
        for status in [Status::Stopped, Status::Unknown, Status::Running] {
            for (stem, (attention, active)) in C_ORDER.iter().rev().zip(FACTS.iter().copied().rev())
            {
                manifest.push(Planted {
                    name: format!("{stem}{}", suffix(status)),
                    status,
                    attention,
                    active,
                    // One degraded row per group, independent of every other
                    // fact, so criterion 5's unknown x degraded pair exists and
                    // criterion 13 can watch degradation survive filtering.
                    degraded: *stem == "Zeta",
                    creation_position: creation,
                });
                creation += 1;
            }
        }
        let mut entries: Vec<SessionEntry> = manifest.iter().map(entry_of).collect();
        order.apply(&mut entries);
        Self { manifest, entries }
    }

    fn world(&self) -> World {
        World::new(NOW, self.entries.clone())
    }

    fn incomplete_world(&self, losses: &[FailedSource]) -> World {
        World::new(NOW, self.entries.clone()).with_losses(losses)
    }

    /// Every planted name whose facts satisfy `keep`, in the order the gate
    /// requires: group order running/unknown/stopped, C-byte within a group.
    fn expected<F: Fn(&Planted) -> bool>(&self, groups: &[Status], keep: F) -> Vec<String> {
        let mut wanted = Vec::new();
        for status in groups {
            let mut group: Vec<&Planted> = self
                .manifest
                .iter()
                .filter(|row| row.status == *status && keep(row))
                .collect();
            group.sort_by(|left, right| left.name.as_bytes().cmp(right.name.as_bytes()));
            wanted.extend(group.into_iter().map(|row| row.name.clone()));
        }
        wanted
    }
}

/// How the reference rows are handed to the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Supply {
    /// As created — reversed stems, stopped first.
    Creation,
    /// The exact reverse of the required output order.
    ReverseRequired,
    /// Statuses interleaved, so no contiguous group exists in the input.
    Interleaved,
    /// Sorted the way a natural-number comparator would.
    NaturalNumber,
    /// Sorted the way a case-folding comparator would.
    CaseFolded,
}

impl Supply {
    const ALL: [Self; 5] = [
        Self::Creation,
        Self::ReverseRequired,
        Self::Interleaved,
        Self::NaturalNumber,
        Self::CaseFolded,
    ];

    fn apply(self, entries: &mut Vec<SessionEntry>) {
        match self {
            Self::Creation => {}
            Self::ReverseRequired => {
                entries.sort_by(|left, right| {
                    group_rank(right.status)
                        .cmp(&group_rank(left.status))
                        .then_with(|| right.name.as_bytes().cmp(left.name.as_bytes()))
                });
            }
            Self::Interleaved => {
                let mut by_group: BTreeMap<usize, Vec<SessionEntry>> = BTreeMap::new();
                for entry in entries.drain(..) {
                    by_group
                        .entry(group_rank(entry.status))
                        .or_default()
                        .push(entry);
                }
                let mut columns: Vec<Vec<SessionEntry>> = by_group.into_values().collect();
                let deepest = columns.iter().map(Vec::len).max().unwrap_or_default();
                for index in 0..deepest {
                    for column in &mut columns {
                        if index < column.len() {
                            entries.push(column[index].clone());
                        }
                    }
                }
            }
            Self::NaturalNumber => {
                entries.sort_by_key(|entry| natural_key(&entry.name));
            }
            Self::CaseFolded => {
                entries.sort_by_key(|entry| entry.name.to_lowercase());
            }
        }
    }
}

fn group_rank(status: Status) -> usize {
    match status {
        Status::Running => 0,
        Status::Unknown => 1,
        Status::Stopped => 2,
    }
}

/// The key a natural-number comparator would sort by: digits as numbers.
fn natural_key(name: &str) -> (String, u64) {
    let digits: String = name.chars().filter(char::is_ascii_digit).collect();
    let letters: String = name.chars().filter(|c| !c.is_ascii_digit()).collect();
    (letters, digits.parse().unwrap_or_default())
}

fn entry_of(row: &Planted) -> SessionEntry {
    let mut entry = if row.degraded {
        SessionEntry::degraded(&row.name, row.status)
    } else {
        SessionEntry::new(&row.name, row.status)
    };
    if row.attention {
        entry.attention = Some(Reason::Blocked);
    }
    if row.active {
        entry.last_active_epoch = Some(NOW.epoch() - 10);
    }
    entry.agents = vec![AgentEntry {
        reference: "cl:lead".to_owned(),
        alias: "cl".to_owned(),
        name: "lead".to_owned(),
        ..AgentEntry::default()
    }];
    entry
}

/// Invoke the real `list`/`ls` surface and return `(stdout, stderr, code)`.
///
/// Criterion 12 forbids a warning that only `--all` sees, and criterion 11
/// binds BOTH command spellings. Neither obligation is on `render` — they are on
/// the surface an operator invokes, and a test that called the renderer directly
/// could not see a filter gate or an alias gap at all. That is the
/// universal-obligation-checked-on-a-particular shape, so these go through the
/// entry point.
fn invoke_over(spelling: &str, flags: &[&str], world: Option<&World>) -> (String, String, u8) {
    let mut argv = vec![spelling.to_owned()];
    argv.extend(flags.iter().map(|flag| (*flag).to_owned()));
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = match ae::run_with(&argv, world, &mut out, &mut err) {
        Ok(code) => code,
        Err(why) => panic!("{spelling} {flags:?}: {why}"),
    };
    let decode = |bytes: Vec<u8>| match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(why) => panic!("output must be utf-8: {why}"),
    };
    (decode(out), decode(err), code)
}

fn args(tokens: &[&str]) -> ListArgs {
    match ListArgs::parse(tokens) {
        Ok(parsed) => parsed,
        Err(unknown) => panic!("these are documented flags: {unknown:?}"),
    }
}

/// The identity sequence a HUMAN listing shows.
///
/// A session row starts at column zero; an agent row is indented. Layout beyond
/// that is an open choice (criterion 15), so nothing here reads a column
/// position or a width.
fn human_rows(text: &str) -> Vec<(String, String)> {
    // These are messages, not zero-width session rows. Match the whole payload
    // rather than a prefix so a future row cannot be silently discarded.
    if matches!(
        text,
        "No running ae sessions. (try: ae list --all)\n"
            | "No recently active sessions.\n"
            | "No running sessions need your attention.\n"
            | "No ae sessions.\n"
            | "No stopped ae sessions.\n"
    ) {
        return Vec::new();
    }
    text.lines()
        .filter(|line| !line.starts_with(char::is_whitespace) && !line.trim().is_empty())
        .map(|line| {
            let mut fields = line.split_whitespace();
            (
                fields.next().unwrap_or_default().to_owned(),
                fields.next().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// The identity sequence a JSON document shows.
fn json_rows(text: &str) -> Vec<(String, String)> {
    let document = match json::parse(text.trim_end()) {
        Ok(document) => document,
        Err(why) => panic!("one complete document: {why:?}"),
    };
    let Some(json::Value::Arr(entries)) = document.get("sessions") else {
        panic!("sessions must be an array");
    };
    entries
        .iter()
        .map(|entry| {
            (
                entry.get_str("name").unwrap_or_default().to_owned(),
                entry.get_str("status").unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

fn names(rows: &[(String, String)]) -> Vec<String> {
    rows.iter().map(|(name, _)| name.clone()).collect()
}

fn parse_json(text: &str) -> json::Value {
    match json::parse(text.trim_end()) {
        Ok(document) => document,
        Err(why) => panic!("one complete document: {why:?}"),
    }
}

/// Object field order is an open choice; arrays stay order-sensitive.
fn json_members_match(left: &json::Value, right: &json::Value) -> bool {
    match (left, right) {
        (json::Value::Obj(left), json::Value::Obj(right)) => {
            left.len() == right.len()
                && left.iter().all(|(key, value)| {
                    right.iter().any(|(other_key, other_value)| {
                        other_key == key && json_members_match(value, other_value)
                    })
                })
        }
        (json::Value::Arr(left), json::Value::Arr(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| json_members_match(left, right))
        }
        (left, right) => left == right,
    }
}

fn without_key(value: &json::Value, key: &str) -> json::Value {
    match value {
        json::Value::Obj(fields) => json::Value::Obj(
            fields
                .iter()
                .filter(|(name, _)| name != key)
                .cloned()
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Every view the phase-3 criteria exercise, as flag lists WITHOUT `--json`.
fn every_human_view() -> Vec<Vec<&'static str>> {
    let mut views = Vec::new();
    for scope in [vec![], vec!["--running"], vec!["--stopped"], vec!["--all"]] {
        for filter in [
            vec![],
            vec!["--needs-attn"],
            vec!["--active"],
            vec!["--needs-attn", "--active"],
        ] {
            let mut view = scope.clone();
            view.extend(filter);
            views.push(view);
        }
    }
    views
}

// ---- criterion 4: the base status views --------------------------------

#[test]
fn criterion_4_the_active_and_history_views_have_the_exact_ratified_domains() {
    let reference = Reference::new(Supply::Creation);
    let world = reference.world();
    for (flags, groups) in [
        (vec![], vec![Status::Running, Status::Unknown]),
        (vec!["--running"], vec![Status::Running, Status::Unknown]),
        (vec!["--stopped"], vec![Status::Stopped]),
        (
            vec!["--all"],
            vec![Status::Running, Status::Unknown, Status::Stopped],
        ),
    ] {
        let wanted = reference.expected(&groups, |_| true);
        let human = human_rows(&render(&args(&flags), &world));
        let mut json_flags = flags.clone();
        json_flags.push("--json");
        let machine = json_rows(&render(&args(&json_flags), &world));

        assert_eq!(names(&human), wanted, "human {flags:?}");
        assert_eq!(names(&machine), wanted, "json {flags:?}");
        // The failure the row names: unknown dropped from an active view.
        if groups.contains(&Status::Unknown) {
            assert!(
                names(&human).iter().any(|name| name.ends_with('U')),
                "{flags:?}: the active view must show unknown sessions"
            );
        }
        assert_eq!(
            names(&human).iter().any(|name| name.ends_with('S')),
            groups.contains(&Status::Stopped),
            "{flags:?}: stopped membership must match the view"
        );
    }
}

// ---- criterion 5: status is rendered literally --------------------------

#[test]
fn criterion_5_every_status_is_spelled_out_and_no_filter_rewrites_one() {
    let reference = Reference::new(Supply::Creation);
    let world = reference.world();
    let human = render(&args(&["--all"]), &world);
    for spelling in ["running", "unknown", "stopped"] {
        assert!(
            human.contains(spelling),
            "the human surface must spell {spelling}: {human}"
        );
    }
    let planted: BTreeMap<String, Status> = reference
        .manifest
        .iter()
        .map(|row| (row.name.clone(), row.status))
        .collect();
    for flags in every_human_view() {
        let mut json_flags = flags.clone();
        json_flags.push("--json");
        for (name, status) in json_rows(&render(&args(&json_flags), &world)) {
            let expected = planted.get(&name).copied().unwrap_or_else(|| {
                panic!("{flags:?}: {name} was rendered but never planted");
            });
            assert_eq!(
                status,
                expected.as_str(),
                "{flags:?}: filtering changed {name}'s status"
            );
        }
        for (name, status) in human_rows(&render(&args(&flags), &world)) {
            let expected = planted.get(&name).copied().unwrap_or_else(|| {
                panic!("{flags:?}: {name} was rendered but never planted");
            });
            assert_eq!(status, expected.as_str(), "{flags:?}: human {name}");
        }
    }
}

// ---- criteria 6, 7, 8: the live-scope filters ---------------------------

#[test]
fn criterion_6_attention_filtering_is_the_positive_fact_on_running_or_unknown() {
    let reference = Reference::new(Supply::Creation);
    let world = reference.world();
    for scope in [vec![], vec!["--running"], vec!["--all"]] {
        let groups = if scope == vec!["--all"] {
            vec![Status::Running, Status::Unknown, Status::Stopped]
        } else {
            vec![Status::Running, Status::Unknown]
        };
        let mut flags = scope.clone();
        flags.push("--needs-attn");
        // Stopped rows carry attention=true too, and must NOT appear.
        let wanted = reference.expected(&groups, |row| {
            row.attention && row.status != Status::Stopped
        });
        assert_eq!(
            names(&human_rows(&render(&args(&flags), &world))),
            wanted,
            "{flags:?}"
        );
        assert!(
            !wanted.is_empty() && wanted.iter().any(|name| name.ends_with('U')),
            "{flags:?}: a matching UNKNOWN row must survive, or the arm proves nothing"
        );
        assert!(
            wanted.len() < reference.manifest.len(),
            "{flags:?}: and the attention=false controls must be excluded"
        );
    }
    for flags in [
        vec!["--stopped", "--needs-attn"],
        vec!["--needs-attn", "--stopped"],
    ] {
        assert!(
            human_rows(&render(&args(&flags), &world)).is_empty(),
            "{flags:?}: a stopped session never satisfies a live-scope predicate"
        );
    }
}

#[test]
fn criterion_7_activity_filtering_is_the_positive_fact_on_running_or_unknown() {
    let reference = Reference::new(Supply::Creation);
    let world = reference.world();
    for scope in [vec![], vec!["--running"], vec!["--all"]] {
        let groups = if scope == vec!["--all"] {
            vec![Status::Running, Status::Unknown, Status::Stopped]
        } else {
            vec![Status::Running, Status::Unknown]
        };
        let mut flags = scope.clone();
        flags.push("--active");
        let wanted = reference.expected(&groups, |row| row.active && row.status != Status::Stopped);
        assert_eq!(
            names(&human_rows(&render(&args(&flags), &world))),
            wanted,
            "{flags:?}"
        );
        assert!(
            wanted.iter().any(|name| name.ends_with('U')),
            "{flags:?}: a matching UNKNOWN row must survive"
        );
    }
    for flags in [vec!["--stopped", "--active"], vec!["--active", "--stopped"]] {
        assert!(
            human_rows(&render(&args(&flags), &world)).is_empty(),
            "{flags:?}: empty in both argument orders"
        );
    }
}

#[test]
fn criterion_8_the_two_filters_intersect_rather_than_uniting() {
    let reference = Reference::new(Supply::Creation);
    let world = reference.world();
    for scope in [vec![], vec!["--running"], vec!["--all"]] {
        let groups = if scope == vec!["--all"] {
            vec![Status::Running, Status::Unknown, Status::Stopped]
        } else {
            vec![Status::Running, Status::Unknown]
        };
        let mut flags = scope.clone();
        flags.extend(["--needs-attn", "--active"]);
        let wanted = reference.expected(&groups, |row| {
            row.attention && row.active && row.status != Status::Stopped
        });
        assert_eq!(
            names(&human_rows(&render(&args(&flags), &world))),
            wanted,
            "{flags:?}"
        );
        // The union would also admit the attention-only and activity-only rows;
        // last-filter-wins would admit one of those two whole classes.
        let union = reference.expected(&groups, |row| {
            (row.attention || row.active) && row.status != Status::Stopped
        });
        assert!(
            wanted.len() < union.len(),
            "{flags:?}: the fixture must distinguish intersection from union"
        );
    }
    for flags in [
        vec!["--stopped", "--needs-attn", "--active"],
        vec!["--active", "--needs-attn", "--stopped"],
    ] {
        assert!(
            human_rows(&render(&args(&flags), &world)).is_empty(),
            "{flags:?}"
        );
    }
}

// ---- criteria 9 and 10: the product owns the order ----------------------

#[test]
fn criterion_10_every_adversarial_supply_order_differs_from_the_required_one() {
    // CALIBRATION FIRST. Criterion 9 means nothing unless the orders it is
    // supposed to defeat actually differ from the answer — an arm whose control
    // agrees is inconclusive, not a pass.
    let required = Reference::new(Supply::Creation)
        .expected(&[Status::Running, Status::Unknown, Status::Stopped], |_| {
            true
        });
    for supply in Supply::ALL {
        let supplied: Vec<String> = Reference::new(supply)
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        assert_ne!(
            supplied, required,
            "{supply:?}: this supply order already equals the required order, so \
             asserting the required order against it would prove nothing"
        );
    }
    // And the two comparators the row names by name are calibrated on the
    // planted stems, independent of supply order.
    let mut natural = C_ORDER.to_vec();
    natural.sort_by_key(|name| natural_key(name));
    assert_ne!(
        natural,
        C_ORDER.to_vec(),
        "natural-number order must differ from C order on these names"
    );
    let mut folded = C_ORDER.to_vec();
    folded.sort_by_key(|name| name.to_lowercase());
    assert_ne!(
        folded,
        C_ORDER.to_vec(),
        "case-folded order must differ from C order on these names"
    );
}

#[test]
fn criterion_9_the_output_order_is_c_byte_within_group_and_running_unknown_stopped() {
    for supply in Supply::ALL {
        let reference = Reference::new(supply);
        let world = reference.world();
        for (flags, groups) in [
            (vec![], vec![Status::Running, Status::Unknown]),
            (vec!["--running"], vec![Status::Running, Status::Unknown]),
            (vec!["--stopped"], vec![Status::Stopped]),
            (
                vec!["--all"],
                vec![Status::Running, Status::Unknown, Status::Stopped],
            ),
            (
                vec!["--all", "--needs-attn"],
                vec![Status::Running, Status::Unknown, Status::Stopped],
            ),
        ] {
            let attention_only = flags.contains(&"--needs-attn");
            let wanted = reference.expected(&groups, |row| {
                !attention_only || (row.attention && row.status != Status::Stopped)
            });
            let human = names(&human_rows(&render(&args(&flags), &world)));
            let mut json_flags = flags.clone();
            json_flags.push("--json");
            let machine = names(&json_rows(&render(&args(&json_flags), &world)));
            assert_eq!(human, wanted, "{supply:?} human {flags:?}");
            assert_eq!(machine, wanted, "{supply:?} json {flags:?}");
        }
    }
}

// ---- criterion 11: the two surfaces agree -------------------------------

#[test]
fn criterion_11_human_and_json_select_and_order_identically() {
    let reference = Reference::new(Supply::ReverseRequired);
    let world = reference.world();
    for spelling in ["list", "ls"] {
        for flags in every_human_view() {
            let (human_text, _, _) = invoke_over(spelling, &flags, Some(&world));
            let mut json_flags = flags.clone();
            json_flags.push("--json");
            let (json_text, _, _) = invoke_over(spelling, &json_flags, Some(&world));
            assert_eq!(
                human_rows(&human_text),
                json_rows(&json_text),
                "{spelling} {flags:?}: the two surfaces disagree"
            );
            // ...and the ALIAS agrees with the primary spelling, which is the
            // half a renderer-level test cannot see at all.
            let (primary, _, _) = invoke_over("list", &flags, Some(&world));
            assert_eq!(
                human_rows(&human_text),
                human_rows(&primary),
                "{spelling} {flags:?}: the alias selected differently"
            );
        }
    }
}

// ---- criterion 12: every human view exposes incompleteness --------------

#[test]
fn criterion_12_every_human_view_warns_with_the_distinct_source_count() {
    let reference = Reference::new(Supply::Creation);
    let one_loss = vec![FailedSource::CanonicalRoot("/home/x/.ae/sessions".into())];
    let two_losses = vec![
        FailedSource::CanonicalRoot("/home/x/.ae/sessions".into()),
        FailedSource::WorktreeRoot("/home/x/.ae/worktrees".into()),
    ];
    // One source recorded TWICE is still one source.
    let repeated = vec![
        FailedSource::CanonicalRoot("/home/x/.ae/sessions".into()),
        FailedSource::CanonicalRoot("/home/x/.ae/sessions".into()),
    ];

    for (losses, expected) in [
        (Vec::new(), None),
        (one_loss, Some(1_usize)),
        (two_losses, Some(2)),
        (repeated, Some(1)),
    ] {
        let world = reference.incomplete_world(&losses);
        let complete = reference.world();
        // EVERY human view, through BOTH spellings, at the real surface.
        for spelling in ["list", "ls"] {
            for flags in every_human_view() {
                let (stdout, stderr, code) = invoke_over(spelling, &flags, Some(&world));
                // Per-surface (gate blob 8cccbe44): incomplete-human rc is
                // open; this loop is human-only. Pin rc only on the complete
                // control.
                if expected.is_none() {
                    assert_eq!(code, 0, "{spelling} {flags:?}");
                }
                match expected {
                    None => assert!(
                        stderr.is_empty(),
                        "{spelling} {flags:?}: a complete snapshot warns about nothing: {stderr}"
                    ),
                    Some(count) => {
                        assert!(
                            !stderr.is_empty(),
                            "{spelling} {flags:?}: this view emitted no warning — a warning \
                             only some views see is one most invocations never get"
                        );
                        assert!(
                            stderr.contains(&count.to_string()),
                            "{spelling} {flags:?}: the warning must carry the COUNT {count}: \
                             {stderr}"
                        );
                    }
                }
                // The found rows never change.
                let (whole, _, _) = invoke_over(spelling, &flags, Some(&complete));
                assert_eq!(
                    human_rows(&stdout),
                    human_rows(&whole),
                    "{spelling} {flags:?}: the found rows changed"
                );
            }
        }
    }
}

#[test]
fn criterion_12_the_count_is_not_hardcoded_because_two_sources_read_two() {
    // The control the criterion names: a boolean or a constant would satisfy the
    // one-loss arm above. Two distinct sources must read `2`, and the same
    // source twice must still read `1`.
    let reference = Reference::new(Supply::Creation);
    let one = reference.incomplete_world(&[FailedSource::CanonicalRoot("/a".into())]);
    let two = reference.incomplete_world(&[
        FailedSource::CanonicalRoot("/a".into()),
        FailedSource::WorktreeRoot("/b".into()),
    ]);
    let (_, one_warning, _) = invoke_over("list", &[], Some(&one));
    let (_, two_warning, _) = invoke_over("list", &[], Some(&two));
    assert_ne!(
        one_warning, two_warning,
        "one loss and two losses must not produce the same warning"
    );
    assert!(one_warning.contains('1') && two_warning.contains('2'));
}

// ---- criterion 13: every JSON document carries completeness -------------

#[test]
fn criterion_13_every_document_carries_version_2_and_the_completeness_boolean() {
    let reference = Reference::new(Supply::Interleaved);
    let losses = vec![FailedSource::WorktreeState(
        "/home/x/.ae/worktrees/a/.ae".into(),
    )];
    for (world, complete) in [
        (reference.world(), true),
        (reference.incomplete_world(&losses), false),
        (World::new(NOW, Vec::new()), true),
        (World::new(NOW, Vec::new()).with_losses(&losses), false),
    ] {
        for (spelling, flags) in ["list", "ls"].into_iter().flat_map(|spelling| {
            every_human_view()
                .into_iter()
                .map(move |view| (spelling, view))
        }) {
            let mut json_flags = flags.clone();
            json_flags.push("--json");
            // Deliberately NOT asserted: whether the machine surface also warns
            // on stderr. Criterion 13 lists JSON stderr warning policy as an
            // OPEN CHOICE, so an implementation that emits the required document
            // AND a warning beside it is correct — and a test rejecting it would
            // fail criterion 15, which fails the gate itself. This build happens
            // not to warn there; that is a choice, not a contract.
            let (text, _stderr, _) = invoke_over(spelling, &json_flags, Some(&world));
            let document = match json::parse(text.trim_end()) {
                Ok(document) => document,
                Err(why) => panic!("{flags:?}: one document: {why:?}"),
            };
            assert_eq!(
                document.get("schema_version"),
                Some(&json::Value::Num(2)),
                "{flags:?}"
            );
            assert_eq!(text.matches("\"schema_version\"").count(), 1, "{flags:?}");
            assert_eq!(
                document.get("inventory_complete"),
                Some(&json::Value::Bool(complete)),
                "{flags:?}"
            );
            assert_eq!(
                text.matches("\"inventory_complete\"").count(),
                1,
                "{flags:?}"
            );
        }
    }
}

#[test]
fn criterion_13_degradation_survives_every_filter_unchanged() {
    let reference = Reference::new(Supply::Creation);
    let world = reference.world();
    let planted: BTreeMap<String, bool> = reference
        .manifest
        .iter()
        .map(|row| (row.name.clone(), row.degraded))
        .collect();
    for flags in every_human_view() {
        let mut json_flags = flags.clone();
        json_flags.push("--json");
        let text = render(&args(&json_flags), &world);
        let document = match json::parse(text.trim_end()) {
            Ok(document) => document,
            Err(why) => panic!("{flags:?}: {why:?}"),
        };
        let Some(json::Value::Arr(entries)) = document.get("sessions") else {
            panic!("sessions must be an array");
        };
        for entry in entries {
            let name = entry.get_str("name").unwrap_or_default().to_owned();
            let degraded = entry.get("degraded") == Some(&json::Value::Bool(true));
            assert_eq!(
                Some(degraded),
                planted.get(&name).copied(),
                "{flags:?}: {name}'s degradation changed under filtering"
            );
        }
    }
}

// ---- criterion 14: completeness is independent of everything else -------

#[test]
fn criterion_14_flipping_completeness_changes_only_the_warning_and_the_boolean() {
    let reference = Reference::new(Supply::Creation);
    let complete = reference.world();
    let incomplete = reference.incomplete_world(&[FailedSource::Server(
        ae::inventory::ServerId::Selected(ae::meta::Selector::Name("gone".to_owned())),
    )]);

    for flags in every_human_view() {
        assert_eq!(
            human_rows(&render(&args(&flags), &complete)),
            human_rows(&render(&args(&flags), &incomplete)),
            "{flags:?}: human selection, status and order moved"
        );
        let mut json_flags = flags.clone();
        json_flags.push("--json");
        let one = parse_json(&render(&args(&json_flags), &complete));
        let other = parse_json(&render(&args(&json_flags), &incomplete));
        assert_eq!(
            one.get("inventory_complete"),
            Some(&json::Value::Bool(true)),
            "{flags:?}"
        );
        assert_eq!(
            other.get("inventory_complete"),
            Some(&json::Value::Bool(false)),
            "{flags:?}"
        );
        assert!(
            json_members_match(
                &without_key(&one, "inventory_complete"),
                &without_key(&other, "inventory_complete"),
            ),
            "{flags:?}: the ONLY difference in the document is the boolean"
        );
    }
    assert!(diagnostic(&complete).is_none());
    assert!(diagnostic(&incomplete).is_some());
}

// ---- criterion 15: the scope guard, turned on this file -----------------

#[test]
fn criterion_15_this_suite_asserts_no_unratified_presentation_choice() {
    // The gate FAILS ITSELF if a test rejects a correct implementation over an
    // open choice. These are the ones phase 3 lists, checked against this
    // file's own source.
    let source = include_str!("phase3.rs");
    let code: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        code.contains("fn criterion_9"),
        "the scan reached the tests"
    );
    for forbidden in [
        // Layout, colour and width are open choices.
        concat!("col", "umn_width"),
        concat!("\\u{1b}", "["),
        // Incomplete-human diagnostic wording is an open choice (criterion 12);
        // the COUNT is not, and `contains(&count.to_string())` is how this
        // file checks it. Per-surface (gate blob 8cccbe44): incomplete-human
        // rc is open, JSON process rc is retained. The grep does not forbid
        // `assert_eq!(code, 0)` because JSON legs must keep that shape; it
        // requires the retained-JSON phrase below so a wholesale drop fails.
        concat!("warning: inv", "entory incomplete"),
        // JSON object field order is an open choice: nothing may compare a
        // whole rendered object to a literal, or treat compact `"k":v` order
        // as the document.
        concat!("\"name\":\"Alpha", "R\",\"status\""),
        concat!("inventory_complete\":", "true"),
    ] {
        assert!(
            !code.contains(forbidden),
            "this suite pins an OPEN CHOICE, which fails the gate itself: {forbidden}"
        );
    }
    // Human bytes are never compared to a literal table.
    assert!(
        !code.contains(concat!("assert_eq!(", "render(")),
        "no test compares a human rendering to expected bytes"
    );
    assert!(
        code.contains("JSON process rc is retained"),
        "criterion 15 retains JSON process rc — dropping every rc pin is too wide"
    );
}

/// A readable dump of the manifest, for a failure message worth reading.
#[allow(dead_code)]
fn describe(reference: &Reference) -> String {
    let mut out = String::new();
    for row in &reference.manifest {
        let _ = writeln!(
            out,
            "{} {} attention={} active={} degraded={} created={}",
            row.name,
            row.status.as_str(),
            row.attention,
            row.active,
            row.degraded,
            row.creation_position
        );
    }
    out
}

// ---- criterion 2: presentation starts from one completed snapshot -------

#[test]
fn criterion_2_presentation_starts_from_one_completed_classified_snapshot() {
    // THE MARKER IS THE PRODUCTION ENTRY POINT, not a line this test appends
    // afterwards. An earlier version called the projection and THEN logged
    // "presentation enter", so everything the projection did happened before the
    // marker and was invisible: a reverse sort inserted inside it still passed.
    // `Presentation::enter` is now the first phase-3 operation, and
    // `at_entry()` is what it received, in the order it received it.
    let root = std::env::temp_dir().join(format!("ae-p3-seq-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for name in ["AlphaR", "ZetaR", "alpha10R"] {
        let dir = root.join("sessions").join(name);
        let written = std::fs::create_dir_all(&dir).and_then(|()| {
            std::fs::write(
                dir.join("meta"),
                // NO SERVER SELECTOR, deliberately. This criterion is about the
                // SEQUENCE — that presentation receives the completed classified
                // set — and its assertions are status-agnostic. Recording a
                // server would make the real route spawn a real `tmux` once the
                // transport exists: a child process, and a dependency on tmux
                // being installed, bought for a query whose ANSWER this test
                // never reads. With no selector the candidates are `unknown`
                // because SC-405l normalizes to `missing`, the route is
                // identical, and the coverage is unchanged.
                "mode=local\nagent.main=cl:lead\n",
            )
        });
        assert!(written.is_ok(), "a planted session");
    }

    // THE REAL ROUTE. `ae::current_world` is what the CLI calls; it returns the
    // classified snapshot beside the world it produced, so both sides of the
    // boundary are observed ON THE PATH THE PRODUCT TAKES. Entering presentation
    // directly here would observe a boundary this test chose — and anything the
    // caller did between classification and entry would be invisible, which is
    // exactly how the previous version passed with a reversal inserted above it.
    let (snapshot, world) = ae::current_world(&root);
    let classified: Vec<(String, &str)> = snapshot
        .sessions
        .iter()
        .map(|c| (c.candidate.name.clone(), c.status.as_str()))
        .collect();
    let presentation = ae::listing::Presentation::enter(&snapshot);
    let at_entry = presentation.at_entry();

    // The world the REAL route produced carries exactly the classified
    // identities — a step between classification and entry would show here.
    let mut produced: Vec<String> = world
        .sessions
        .iter()
        .map(|entry| entry.name.clone())
        .collect();
    produced.sort();
    let mut expected: Vec<String> = classified.iter().map(|(name, _)| name.clone()).collect();
    expected.sort();
    assert_eq!(
        produced, expected,
        "the world the CLI route built does not carry the classified set"
    );

    assert_eq!(
        at_entry, classified,
        "the presentation input is the completed classified set, IN ITS ORDER — a \
         filter or sort at or before the boundary would show here"
    );
    assert_eq!(at_entry.len(), 3, "and the fixture is not empty");

    // Everything downstream presents from that one input and changes nothing.
    for spelling in ["list", "ls"] {
        for flags in every_human_view() {
            let (stdout, _, _) = invoke_over(spelling, &flags, Some(&world));
            for (name, status) in human_rows(&stdout) {
                assert!(
                    at_entry
                        .iter()
                        .any(|(known, known_status)| *known == name && *known_status == status),
                    "{spelling} {flags:?}: {name} was presented with facts the input never held"
                );
            }
        }
    }
    assert_eq!(
        presentation.at_entry(),
        at_entry,
        "and the input itself is unchanged after every surface has read it"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A backend whose every query fails — the phase-2 seam, held fixed.
struct Down;

impl ae::inventory::Discovery for Down {
    fn enumerate(
        &self,
        _server: &ae::inventory::ServerId,
    ) -> Result<Vec<ae::inventory::DiscoveredSession>, ae::inventory::QueryFailed> {
        Err(ae::inventory::QueryFailed)
    }
}

// ---- criterion 3: presentation output does not re-derive any planted snapshot fact ---
//
// Live gate blob 8cccbe44: one fixed snapshot, two opposed external worlds AFTER
// `presentation enter`, through the real `ae::current_world` → `Presentation::enter`
// → `list`/`ls` route. An injected `World` never observed those bytes, so a
// re-derivation on the real path was invisible.

/// One row's planted external facts. Attention and activity ride the event
/// stream; goal and mode ride the durable `meta`.
#[derive(Clone, Copy)]
struct ExternalFacts {
    name: &'static str,
    attention: bool,
    active: bool,
    goal: &'static str,
    mode: &'static str,
}

/// World A: four independent fact pairs, C-order names, canonical layout.
const WORLD_A: [ExternalFacts; 4] = [
    ExternalFacts {
        name: "AlphaR",
        attention: true,
        active: true,
        goal: "alpha-a",
        mode: "local",
    },
    ExternalFacts {
        name: "ZetaR",
        attention: false,
        active: true,
        goal: "zeta-a",
        mode: "local",
    },
    ExternalFacts {
        name: "alpha10R",
        attention: true,
        active: false,
        goal: "ten-a",
        mode: "local",
    },
    ExternalFacts {
        name: "alpha9R",
        attention: false,
        active: false,
        goal: "nine-a",
        mode: "local",
    },
];

/// World B: every planted fact opposed, same identities.
const WORLD_B: [ExternalFacts; 4] = [
    ExternalFacts {
        name: "AlphaR",
        attention: false,
        active: false,
        goal: "alpha-b",
        mode: "copy",
    },
    ExternalFacts {
        name: "ZetaR",
        attention: true,
        active: false,
        goal: "zeta-b",
        mode: "copy",
    },
    ExternalFacts {
        name: "alpha10R",
        attention: false,
        active: true,
        goal: "ten-b",
        mode: "copy",
    },
    ExternalFacts {
        name: "alpha9R",
        attention: true,
        active: true,
        goal: "nine-b",
        mode: "copy",
    },
];

/// Extra identities planted AFTER enter so a re-walk changes durable order.
const ORDER_EXTRAS: [&str; 2] = ["AAA", "zzz"];

fn c3_root(tag: &str) -> PathBuf {
    let root = PathBuf::from(format!("/tmp/ae-p3-c3-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        std::fs::create_dir_all(root.join("sessions")).is_ok(),
        "a scratch state root"
    );
    root
}

fn event_line(offset: i64, action: &str, extra: &str) -> String {
    format!(
        r#"{{"ts":"{}","actor":"cl:lead","action":"{action}"{extra}}}"#,
        Timestamp::from_epoch(NOW.epoch() + offset)
    )
}

fn write_session(dir: &Path, facts: ExternalFacts) {
    assert!(
        std::fs::create_dir_all(dir).is_ok(),
        "session dir {}",
        facts.name
    );
    let meta = format!(
        "mode={}\nagent.main=cl:lead\ngoal={}\n",
        facts.mode, facts.goal
    );
    assert!(
        std::fs::write(dir.join("meta"), meta).is_ok(),
        "meta {}",
        facts.name
    );
    // Recent events sit well inside the 300s activity window of NOW and NOW+1.
    // Old events sit far outside it, so the opposed clock cannot flip --active.
    let events = match (facts.attention, facts.active) {
        (true, true) => format!("{}\n", event_line(-10, "state", r#","ref":"blocked""#)),
        (false, true) => format!("{}\n", event_line(-10, "done", "")),
        (true, false) => format!("{}\n", event_line(-10_000, "state", r#","ref":"blocked""#)),
        (false, false) => String::new(),
    };
    if events.is_empty() {
        let _ = std::fs::remove_file(dir.join("events.jsonl"));
    } else {
        assert!(
            std::fs::write(dir.join("events.jsonl"), events).is_ok(),
            "events {}",
            facts.name
        );
    }
}

fn plant(root: &Path, world: &[ExternalFacts]) {
    for facts in world {
        write_session(&root.join("sessions").join(facts.name), *facts);
    }
}

fn record_paths(snapshot: &ae::liveness::Snapshot) -> Vec<PathBuf> {
    snapshot
        .sessions
        .iter()
        .map(|classified| {
            classified.candidate.durable.as_ref().map_or_else(
                || {
                    panic!(
                        "{} has no durable path the product observed",
                        classified.candidate.name
                    )
                },
                |record| record.path.clone(),
            )
        })
        .collect()
}

fn durable_names(root: &Path) -> Vec<String> {
    durable_records(&Roots::under(root))
        .records
        .into_iter()
        .map(|record| record.name)
        .collect()
}

/// Prove the planted facts through the primitives that originally supplied them.
fn assert_facts_via_product(paths: &[PathBuf], world: &[ExternalFacts]) {
    let runtime = SessionRuntime::new(Status::Unknown);
    for facts in world {
        let dir = paths
            .iter()
            .find(|path| path.file_name().is_some_and(|name| name == facts.name))
            .unwrap_or_else(|| panic!("the product never observed {}", facts.name));

        // (a) event/attention bytes — `SessionRead::open` is the event-store
        // primitive; `entry_for` is how those bytes become listing facts.
        let events = SessionRead::open(dir);
        if facts.active || facts.attention {
            assert!(
                events.as_ref().is_ok_and(|read| !read.events.is_empty()),
                "{}: event bytes must be readable through SessionRead::open",
                facts.name
            );
        }
        let entry = entry_for(dir, facts.name, &runtime, NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(
            entry.needs_attention(),
            facts.attention,
            "{}: attention through entry_for",
            facts.name
        );
        let active = entry.last_active_epoch.is_some_and(|epoch| {
            Timestamp::from_epoch(epoch).seconds_until(NOW) <= DEFAULT_ACTIVE_WINDOW_SECS
        });
        assert_eq!(
            active, facts.active,
            "{}: activity through entry_for",
            facts.name
        );

        // (b) durable session bytes — `Meta::read` is the meta primitive.
        let meta = match Meta::read(dir) {
            Ok(meta) => meta,
            Err(why) => panic!("{}: Meta::read: {why}", facts.name),
        };
        assert_eq!(
            meta.goal(),
            Some(facts.goal),
            "{}: goal through Meta::read",
            facts.name
        );
        assert_eq!(
            meta.mode(),
            Some(facts.mode),
            "{}: mode through Meta::read",
            facts.name
        );
        assert_eq!(
            entry.goal.as_deref(),
            Some(facts.goal),
            "{}: goal through entry_for",
            facts.name
        );
        assert_eq!(
            entry.mode.as_deref(),
            Some(facts.mode),
            "{}: mode through entry_for",
            facts.name
        );
    }
}

/// Identity + status per view, human and JSON. Filtering membership and order
/// live in the row sequence; durable fields ride the JSON payload below.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PresentedView {
    label: String,
    human: Vec<(String, String)>,
    machine: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlantedPayload {
    name: String,
    needs_attention: bool,
    last_active_epoch: Option<i64>,
    goal: Option<String>,
    mode: Option<String>,
}

impl PlantedPayload {
    fn recently_active(&self) -> bool {
        self.last_active_epoch.is_some_and(|epoch| {
            Timestamp::from_epoch(epoch).seconds_until(NOW) <= DEFAULT_ACTIVE_WINDOW_SECS
        })
    }
}

fn presented(world: &World) -> Vec<PresentedView> {
    let mut out = Vec::new();
    for spelling in ["list", "ls"] {
        for flags in every_human_view() {
            // Per-surface (gate blob 8cccbe44): human rc unpinned; JSON
            // process rc is retained.
            let (human, _, _) = invoke_over(spelling, &flags, Some(world));
            let mut json_flags = flags.clone();
            json_flags.push("--json");
            let (machine, _, json_code) = invoke_over(spelling, &json_flags, Some(world));
            assert_eq!(
                json_code, 0,
                "{spelling} {json_flags:?}: JSON process rc is retained"
            );
            out.push(PresentedView {
                label: format!("{spelling} {flags:?}"),
                human: human_rows(&human),
                machine: json_rows(&machine),
            });
        }
    }
    out
}

fn planted_payload(world: &World) -> Vec<PlantedPayload> {
    let (text, _, _) = invoke_over("list", &["--all", "--json"], Some(world));
    let document = match json::parse(text.trim_end()) {
        Ok(document) => document,
        Err(why) => panic!("one document: {why:?}"),
    };
    let Some(json::Value::Arr(entries)) = document.get("sessions") else {
        panic!("sessions must be an array");
    };
    entries
        .iter()
        .map(|entry| PlantedPayload {
            name: entry.get_str("name").unwrap_or_default().to_owned(),
            needs_attention: entry.get("needs_attention") == Some(&json::Value::Bool(true)),
            last_active_epoch: match entry.get("last_active_epoch") {
                Some(json::Value::Num(epoch)) => Some(*epoch),
                _ => None,
            },
            goal: entry.get_str("goal").map(ToOwned::to_owned),
            mode: entry.get_str("mode").map(ToOwned::to_owned),
        })
        .collect()
}

fn project(snapshot: &ae::liveness::Snapshot, now: Timestamp) -> World {
    ae::listing::Presentation::enter(snapshot).world(now, DEFAULT_UNANSWERED_SECS)
}

/// Oppose every planted axis under `root`. Called from the after-classify hook
/// so the disk changes on `current_world`'s path, not after it returns.
fn oppose_external_world(root: &Path) {
    for facts in WORLD_B {
        write_session(&root.join("sessions").join(facts.name), facts);
    }
    for name in ORDER_EXTRAS {
        write_session(
            &root.join("sessions").join(name),
            ExternalFacts {
                name,
                attention: false,
                active: false,
                goal: "extra",
                mode: "local",
            },
        );
    }
}

struct AfterClassifyHook;

impl AfterClassifyHook {
    fn arm() -> Self {
        ae::set_after_classify_hook(Some(oppose_external_world));
        Self
    }
}

impl Drop for AfterClassifyHook {
    fn drop(&mut self) {
        ae::set_after_classify_hook(None);
    }
}

fn snapshot_names(snapshot: &ae::liveness::Snapshot) -> Vec<String> {
    snapshot
        .sessions
        .iter()
        .map(|classified| classified.candidate.name.clone())
        .collect()
}

#[test]
fn criterion_3_presentation_output_does_not_rederive_any_planted_snapshot_fact() {
    // THE REAL ROUTE is `current_world`: classify, then Presentation::enter.
    // Mutating the disk AFTER that function returns and calling enter on the
    // carried snapshot is below the list/ls caller — a reread between classify
    // and enter is invisible. The opposed world is planted by a hook that
    // `current_world` itself runs in that window.
    let root = c3_root("fixed");
    plant(&root, &WORLD_A);

    let (snapshot_a, world_a) = ae::current_world(&root);
    let paths = record_paths(&snapshot_a);
    assert_eq!(paths.len(), 4, "the fixture reached the real route");
    assert_facts_via_product(&paths, &WORLD_A);
    let order_a = durable_names(&root);
    assert_eq!(
        order_a,
        ["AlphaR", "ZetaR", "alpha10R", "alpha9R"],
        "canonical path order is the product's durable-records primitive"
    );

    let before = presented(&world_a);
    let payload_a = planted_payload(&world_a);
    let names_a = snapshot_names(&snapshot_a);

    let (snapshot_b, world_b) = {
        let _hook = AfterClassifyHook::arm();
        ae::current_world(&root)
    };

    assert_eq!(
        snapshot_names(&snapshot_b),
        names_a,
        "classification ran on world A; extras must not be in the fixed snapshot"
    );
    assert_facts_via_product(&paths, &WORLD_B);
    let order_b = durable_names(&root);
    assert_ne!(
        order_a, order_b,
        "durable traversal order must actually change, through durable_records"
    );
    assert!(
        order_b.first().is_some_and(|name| name == "AAA")
            && order_b.last().is_some_and(|name| name == "zzz"),
        "the extras bookend the walk: {order_b:?}"
    );

    let after = presented(&world_b);
    let payload_b = planted_payload(&world_b);
    assert_eq!(
        before, after,
        "an opposed external fact reached the presentation of a fixed snapshot"
    );
    assert_eq!(
        payload_a, payload_b,
        "durable goal/mode/attention/activity bytes reached the JSON surface"
    );

    // Opposed clock. NOW is even; NOW+1 is odd. Both keep every active row
    // inside the 300s window and every inactive row outside it. Clock is a
    // world() parameter after enter, so this arm re-projects the SAME snapshot.
    let clock_base = presented(&project(&snapshot_b, NOW));
    let world_clock = project(&snapshot_b, Timestamp::from_epoch(NOW.epoch() + 1));
    assert_eq!(
        presented(&world_clock),
        clock_base,
        "an opposed clock changed selection, status, filtering or order"
    );
    assert_eq!(
        planted_payload(&world_clock),
        payload_a,
        "an opposed clock rewrote planted payload fields"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn criterion_3_a_new_listing_through_the_real_route_sees_every_opposed_axis() {
    // CALIBRATION. The arm above is a non-difference. A non-difference is
    // vacuous unless a NEW listing on the same real route, after the same
    // opposed plant, actually moves — per axis, through the primitives that
    // supplied the facts. If this fails, the main arm's agreement proves nothing.
    let root = c3_root("fresh");
    plant(&root, &WORLD_A);
    let (snapshot_a, world_a) = ae::current_world(&root);
    let paths = record_paths(&snapshot_a);
    let before = presented(&world_a);
    let payload_a = planted_payload(&world_a);
    let attn_a: Vec<String> = payload_a
        .iter()
        .filter(|row| row.needs_attention)
        .map(|row| row.name.clone())
        .collect();
    let active_a: Vec<String> = payload_a
        .iter()
        .filter(|row| row.recently_active())
        .map(|row| row.name.clone())
        .collect();
    let goals_a: Vec<Option<String>> = payload_a.iter().map(|row| row.goal.clone()).collect();
    let order_a = durable_names(&root);

    for facts in WORLD_B {
        let dir = paths
            .iter()
            .find(|path| path.file_name().is_some_and(|name| name == facts.name))
            .unwrap_or_else(|| panic!("no observed path for {}", facts.name));
        write_session(dir, facts);
    }
    for name in ORDER_EXTRAS {
        write_session(
            &root.join("sessions").join(name),
            ExternalFacts {
                name,
                attention: false,
                active: false,
                goal: "extra",
                mode: "local",
            },
        );
    }

    let (snapshot_b, world_b) = ae::current_world(&root);
    let after = presented(&world_b);
    let payload_b = planted_payload(&world_b);
    let attn_b: Vec<String> = payload_b
        .iter()
        .filter(|row| row.needs_attention)
        .map(|row| row.name.clone())
        .collect();
    let active_b: Vec<String> = payload_b
        .iter()
        .filter(|row| row.recently_active())
        .map(|row| row.name.clone())
        .collect();
    let goals_b: Vec<Option<String>> = payload_b.iter().map(|row| row.goal.clone()).collect();
    let order_b = durable_names(&root);
    let names_b: Vec<String> = snapshot_b
        .sessions
        .iter()
        .map(|classified| classified.candidate.name.clone())
        .collect();

    assert_ne!(
        attn_a, attn_b,
        "(a) attention: a new listing must see the flipped event facts"
    );
    assert_ne!(
        active_a, active_b,
        "(a) activity: a new listing must see the flipped event facts"
    );
    assert_ne!(
        goals_a, goals_b,
        "(b) durable bytes: a new listing must see the flipped meta"
    );
    assert_ne!(
        order_a, order_b,
        "(c) durable order: durable_records must see the extras"
    );
    assert!(
        names_b.iter().any(|name| name == "AAA") && names_b.iter().any(|name| name == "zzz"),
        "(c) current_world must observe the extras: {names_b:?}"
    );
    assert_ne!(
        before, after,
        "the differential cannot see a changed fact, so the main arm's non-difference proves nothing"
    );

    let _ = std::fs::remove_dir_all(&root);
}

// ---- criterion 10: the locale arm, with a demonstrated control ----------

#[test]
fn criterion_10_a_non_c_locale_collates_these_names_differently_and_output_does_not() {
    // The product cannot consult a locale — Rust compares `str` by bytes — but
    // the arm is only meaningful if a locale that WOULD disagree exists and is
    // demonstrated to disagree on these exact names. An arm whose control agrees
    // is inconclusive, not a pass, so this fails loudly rather than passing when
    // no such locale is available.
    let scratch = std::env::temp_dir().join(format!("ae-p3-loc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    assert!(std::fs::create_dir_all(&scratch).is_ok(), "a scratch dir");
    let names_file = scratch.join("names");
    let planted = C_ORDER
        .iter()
        .map(|stem| format!("{stem}R"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        std::fs::write(&names_file, format!("{planted}\n")).is_ok(),
        "the planted names"
    );

    let sort_under = |locale: &str| -> Option<Vec<String>> {
        let out = scratch.join(format!("out-{locale}"));
        let err = scratch.join(format!("err-{locale}"));
        let invocation = Invocation::new("sort")
            .arg(names_file.display().to_string())
            .env("LC_ALL", locale);
        let status = raw::run(&invocation, &scratch, &out, &err).ok()?;
        if !matches!(status.outcome(), ExitOutcome::Code(0)) {
            return None;
        }
        Some(
            std::fs::read_to_string(&out)
                .ok()?
                .lines()
                .map(ToOwned::to_owned)
                .collect(),
        )
    };

    let c_order = sort_under("C");
    let other = ["en_US.UTF-8", "en_GB.UTF-8", "de_DE.UTF-8"]
        .into_iter()
        .find_map(|locale| sort_under(locale).map(|order| (locale, order)));
    let _ = std::fs::remove_dir_all(&scratch);

    let c_order = c_order.unwrap_or_else(|| panic!("`sort` under LC_ALL=C must be runnable"));
    let (locale, collated) = other.unwrap_or_else(|| {
        panic!("this arm needs a non-C locale to be available to be meaningful")
    });
    assert_ne!(
        collated, c_order,
        "{locale} collates these names the same as C, so this arm is INCONCLUSIVE \
         rather than a pass — pick names or a locale that disagree"
    );

    // The product's answer is the C one, and it did not consult anything.
    let reference = Reference::new(Supply::CaseFolded);
    let (stdout, _, _) = invoke_over("list", &["--running"], Some(&reference.world()));
    // The active view carries running THEN unknown (SC-017m), so the running
    // group is its prefix — and that prefix must be exactly what `sort` under
    // LC_ALL=C produced for the same names.
    let shown = names(&human_rows(&stdout));
    let running_group: Vec<String> = shown
        .iter()
        .filter(|name| name.ends_with('R'))
        .cloned()
        .collect();
    assert_eq!(
        running_group, c_order,
        "the running group must be exactly `sort`'s C order for the same names"
    );
    assert_eq!(
        shown
            .iter()
            .take(c_order.len())
            .cloned()
            .collect::<Vec<_>>(),
        c_order,
        "and it is the PREFIX, because running precedes unknown"
    );
}

// ---- criterion 3: the access boundary, asked of the compiler ------------

/// Every `(file, line)` where this crate may read the outside world, as CLIPPY
/// reports it under `--force-warn`.
///
/// Asked of the compiler rather than of the source text, because the thing being
/// bounded is a CAPABILITY and not a spelling: `Path::exists` is a filesystem
/// observation under none of the names a reader would think to grep for, and an
/// alias or UFCS call defeats a text scan entirely. `--force-warn` cannot be
/// silenced by any `allow`, `expect` or crate-root attribute, so the doors it
/// reports are the doors that exist.
fn world_reading_sites() -> Vec<(String, usize)> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let out = std::env::temp_dir().join(format!("ae-p3-clippy-{}", std::process::id()));
    let err = out.with_extension("err");
    let invocation = Invocation::new(cargo)
        .arg("clippy")
        .arg("--quiet")
        .arg("--locked")
        .arg("--all-targets")
        .arg("--all-features")
        .arg("--message-format=json")
        .arg("--target-dir")
        .arg(
            manifest
                .join("target")
                .join("world-read-guard")
                .display()
                .to_string(),
        )
        .arg("--")
        .arg("--force-warn")
        .arg("clippy::disallowed_methods");
    let status = raw::run(&invocation, manifest, &out, &err);
    assert!(status.is_ok(), "this guard needs cargo and clippy on PATH");

    let stdout = std::fs::read_to_string(&out).unwrap_or_default();
    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_file(&err);
    let mut sites = Vec::new();
    for line in stdout.lines() {
        if !line.starts_with('{') {
            continue;
        }
        let Ok(value) = json::parse(line) else {
            continue;
        };
        if value.get_str("reason") != Some("compiler-message") {
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        let is_boundary = message.get_str("code_text") == Some("clippy::disallowed_methods")
            || message.get("code").and_then(|code| code.get_str("code"))
                == Some("clippy::disallowed_methods");
        if is_boundary && let Some(json::Value::Arr(spans)) = message.get("spans") {
            for span in spans {
                let (Some(file), Some(json::Value::Num(line_no))) =
                    (span.get_str("file_name"), span.get("line_start"))
                else {
                    continue;
                };
                if let Ok(line_no) = usize::try_from(*line_no) {
                    sites.push((file.to_owned(), line_no));
                }
            }
        }
    }
    sites.sort();
    sites.dedup();
    sites
}

/// Whether `line` in `file` is PRODUCT code rather than a test module.
fn is_product_line(file: &str, line: usize) -> bool {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return false;
    };
    match text.find("#[cfg(test)]") {
        Some(at) => text[..at].lines().count() >= line,
        None => true,
    }
}

#[test]
fn criterion_3_the_places_this_crate_can_read_the_world_are_the_inventoried_ones() {
    // A TRIPWIRE OVER ENTRY POINTS, AND ITS LIMIT IS THE TEST. `clippy.toml`
    // names eleven resolved paths; safe std still exposes `canonicalize`,
    // `read_link`, `symlink_metadata`, `OpenOptions::open` and `DirEntry`
    // observations, and a discarded call to any of them slips straight past.
    // The empty dependency tables and `unsafe_code = "forbid"` close the
    // THIRD-PARTY and LIBC routes; neither closes an unlisted safe-std one, and
    // reading them as covering the enumeration is what let an earlier version of
    // this file call a name list a boundary.
    //
    // The live criterion 3 residual is explicit: a source-name inventory is NOT
    // a capability boundary and makes no zero-access claim. This test stays as
    // the cheap early warning for the eleven named doors, and claims only them.
    let sites = world_reading_sites();
    assert!(
        !sites.is_empty(),
        "the force-warn probe found no world-reading call anywhere; it did not run"
    );

    let mut product: Vec<String> = sites
        .iter()
        .filter(|(file, line)| file.starts_with("src/") && is_product_line(file, *line))
        .map(|(file, _)| file.clone())
        .collect();
    product.sort();
    product.dedup();

    assert_eq!(
        product,
        vec![
            // The OPAQUE event-container read and existence test, shared by the
            // `requests` and `events-tail` surfaces. One file rather than two:
            // both surfaces read the same container the same quiet way, so the
            // read sits with the framing in `event_text` and neither surface
            // module opens anything itself.
            "src/event_text.rs".to_owned(),
            "src/events.rs".to_owned(),
            "src/inventory.rs".to_owned(),
            "src/lib.rs".to_owned(),
            // The memo file read behind `memo read`/`memo tail`. Registered
            // deliberately: the file is the helper's own, but reading it is a
            // new door all the same.
            "src/memo.rs".to_owned(),
            "src/meta.rs".to_owned(),
            // The git branch read: `HEAD` under the session's own work tree,
            // plus the `.git` pointer file a worktree uses instead of a
            // directory. Registered deliberately — the branch is a fact about
            // the world and reading it is a new door, so it belongs on this
            // list rather than being routed around the tripwire that named it.
            // It reads git's files instead of launching `git` so that `list`
            // stays a no-subprocess path and still works with no git installed;
            // every failure is `None`, so a listing never fails on it.
            "src/session.rs".to_owned(),
        ],
        "the set of places product code can reach the eleven named entry points changed"
    );
}

#[test]
fn the_presentation_input_declares_no_path_typed_field() {
    // WHAT THIS ASSERTS, AND IT IS NOT WHAT ITS PREDECESSOR CLAIMED. It checks
    // one fact: neither `World` nor `SessionEntry` DECLARES a path-typed field.
    // That is true, and it is worth keeping — a path field would be an address
    // handed to presentation for free.
    //
    // IT DOES NOT MEAN PRESENTATION CANNOT ADDRESS AE'S STATE, and the previous
    // version of this test said exactly that. colead disproved it twice: a
    // `type StateAddress = PathBuf` alias defeats a text scan for `PathBuf`, and
    // — with no new field at all — `render` can COMPOSE the SC-400d
    // worktree-nested record path from `SessionEntry.work_dir` plus `.ae` plus
    // the session name. `work_dir` is payload contractually and an address
    // operationally, and no scan of field TYPES can see that.
    //
    // So this is a narrow structural fact, not a boundary. Live criterion 3
    // does not claim zero post-boundary access; this test does not pretend to.
    let module = product_module("listing.rs");
    let world = module
        .split_once("pub struct World {")
        .and_then(|(_, rest)| rest.split_once('}'))
        .map_or_else(
            || panic!("World must be declared in listing.rs"),
            |(body, _)| body.to_owned(),
        );
    for declared in ["PathBuf", "&Path", "&'a Path"] {
        assert!(
            !world.contains(declared),
            "the presentation input declares a path-typed field ({declared}): {world}"
        );
    }
    assert!(
        world.contains("losses: usize"),
        "SC-017o's fact crosses as a COUNT, which is what removed the last declared path"
    );
}

/// One product module's source, comments stripped, tests excluded.
fn product_module(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name);
    let Ok(text) = std::fs::read_to_string(&path) else {
        panic!("{name} must be readable");
    };
    let code: String = text
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    code.split_once("#[cfg(test)]")
        .map_or(code.clone(), |(module, _)| module.to_owned())
}

// ---- SC-017p/q/r + SC-509e: three-valued agent liveness ----------------
//
// RESTORED. These four were deleted by a slice-to-end-of-file in the phase-3
// blocker fix (1f92bca2), which replaced everything after the criterion-3 tests
// because they had been appended below them. The suite went 451 to 448 and the
// drop was reported as green. A test count that FALLS after a change described
// as adding one is a contradiction; that number is now part of every report.

#[test]
fn sc_509e_the_agent_liveness_field_is_present_even_when_null() {
    // The closed JSON domain is `true | false | null`, and PRESENT is the part
    // that matters: a consumer gating on the key must not have to tell "absent
    // because unknown" from "absent because the writer is old".
    for (alive, expected) in [
        (Some(true), json::Value::Bool(true)),
        (Some(false), json::Value::Bool(false)),
        (None, json::Value::Null),
    ] {
        let mut entry = SessionEntry::new("s", Status::Running);
        entry.agents = vec![AgentEntry {
            reference: "cl:lead".to_owned(),
            alias: "cl".to_owned(),
            name: "lead".to_owned(),
            alive,
            ..AgentEntry::default()
        }];
        let world = World::new(NOW, vec![entry]);
        let (text, _, _) = invoke_over("list", &["--all", "--json"], Some(&world));
        let document = match json::parse(text.trim_end()) {
            Ok(document) => document,
            Err(why) => panic!("one document: {why:?}"),
        };
        let Some(json::Value::Arr(sessions)) = document.get("sessions") else {
            panic!("sessions must be an array");
        };
        let Some(json::Value::Arr(agents)) = sessions[0].get("agents") else {
            panic!("agents must be an array");
        };
        assert_eq!(
            agents[0].get("alive"),
            Some(&expected),
            "alive={alive:?} must render as {expected:?}"
        );
        assert!(
            text.contains(r#""alive":"#),
            "the key is present in every document: {text}"
        );
    }
}

/// The health cell the product renders for an agent with this liveness.
///
/// Read FROM the product rather than written down: SC-017r leaves the words or
/// glyphs an OPEN CHOICE, so a test demanding the string "unknown" would reject
/// a correct glyph renderer, and criterion 15 fails the gate itself for that.
fn health_cell(alive: Option<bool>) -> String {
    let mut entry = SessionEntry::new("s", Status::Running);
    entry.agents = vec![AgentEntry {
        reference: "cl:lead".to_owned(),
        alias: "cl".to_owned(),
        name: "lead".to_owned(),
        alive,
        ..AgentEntry::default()
    }];
    let world = World::new(NOW, vec![entry]);
    let (machine, _, _) = invoke_over("list", &["--all", "--json"], Some(&world));
    // HEALTH IS A JSON MEMBER, NOT A TABLE COLUMN — and that is a deliberate
    // change, not an omission. Nothing on the list path fills per-agent health,
    // so the column rendered `unknown` for every agent of every session; an
    // always-unknown column is not a three-way distinction, it is noise sitting
    // where a reader looks for state. The three-way distinction this gate exists
    // to protect is real and still enforced — here, on the member that carries
    // it. Restore the column together with a pane query that populates it.
    let member = machine
        .split(r#""alive":"#)
        .nth(1)
        .unwrap_or_else(|| panic!("the alive member must render for alive={alive:?}"));
    member
        .chars()
        .take_while(|c| c.is_alphanumeric())
        .collect::<String>()
}

#[test]
fn sc_017r_the_three_agent_healths_are_distinct_and_none_is_empty() {
    let alive = health_cell(Some(true));
    let dead = health_cell(Some(false));
    let unknown = health_cell(None);

    for (cell, which) in [(&alive, "alive"), (&dead, "dead"), (&unknown, "unknown")] {
        assert!(
            !cell.is_empty(),
            "{which} must not render as blank — frozen bash rendered a failed query \
             exactly like a healthy agent, and that silence is the defect"
        );
    }
    assert_ne!(alive, dead, "alive and dead must be distinguishable");
    assert_ne!(dead, unknown, "dead and unknown must be distinguishable");
    assert_ne!(alive, unknown, "alive and unknown must be distinguishable");
}

#[test]
fn sc_017q_an_unknown_agent_keeps_its_declared_state_and_reason() {
    // Liveness null never nulls or relabels an independently known fact.
    let mut entry = SessionEntry::new("s", Status::Running);
    entry.agents = vec![AgentEntry {
        reference: "cl:lead".to_owned(),
        alias: "cl".to_owned(),
        name: "lead".to_owned(),
        alive: None,
        state: Some("blocked".to_owned()),
        reason: Some(ae::attention::Reason::Blocked),
        session_id: Some("e795c9e9".to_owned()),
    }];
    let world = World::new(NOW, vec![entry]);
    let (text, _, _) = invoke_over("list", &["--all", "--json"], Some(&world));
    let document = match json::parse(text.trim_end()) {
        Ok(document) => document,
        Err(why) => panic!("one document: {why:?}"),
    };
    let Some(json::Value::Arr(sessions)) = document.get("sessions") else {
        panic!("sessions must be an array");
    };
    let Some(json::Value::Arr(agents)) = sessions[0].get("agents") else {
        panic!("agents must be an array");
    };
    assert_eq!(agents[0].get("alive"), Some(&json::Value::Null));
    assert_eq!(agents[0].get_str("state"), Some("blocked"));
    assert_eq!(agents[0].get_str("reason"), Some("blocked"));
    assert_eq!(agents[0].get_str("session_id"), Some("e795c9e9"));
}

#[test]
fn sc_017q_the_entry_point_reports_unknown_agents_rather_than_dead_ones() {
    // END TO END, and the reason it holds is the INJECTION, not the build. An
    // earlier version of this comment said "this build has no transport, so no
    // pane can be observed" — a claim about the product, which stopped being
    // true the moment a real transport landed, while the test kept passing for
    // an entirely different reason and would have taught the next reader the
    // wrong one.
    //
    // What actually holds it: `Down` fixes the SESSION transport, and no pane
    // observation exists at any transport yet (SC-017p's positive and negative
    // proofs are unbuilt), so this observes the agent surface alone. When a pane
    // transport lands, THIS comment is the one that goes stale next, and the
    // repair is the same — name what the test injects, never what the build
    // happens to lack.
    let root = std::env::temp_dir().join(format!("ae-p3-agents-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join("sessions").join("AlphaR");
    let written = std::fs::create_dir_all(&dir).and_then(|()| {
        std::fs::write(
            dir.join("meta"),
            "mode=local\nagent.main=cl:lead\ntmux_server_kind=name\ntmux_server=B\n",
        )
    });
    assert!(written.is_ok(), "a planted session");

    let scan = ae::inventory::durable_records(&ae::inventory::Roots::under(&root));
    let snapshot = ae::liveness::classify(ae::inventory::take(scan, None, &Down), &Down);
    let world = ae::listing::Presentation::enter(&snapshot)
        .world(NOW, ae::session::DEFAULT_UNANSWERED_SECS);
    let (human, _, _) = invoke_over("list", &["--all"], Some(&world));
    let (machine, _, _) = invoke_over("list", &["--all", "--json"], Some(&world));
    let _ = std::fs::remove_dir_all(&root);

    // Bound to the product's own choice, not to a word this test picked.
    let null_cell = health_cell(None);
    let dead_cell = health_cell(Some(false));
    assert_ne!(null_cell, dead_cell, "unknown is not dead in the digest");
    let agent_row = human
        .lines()
        .find(|line| line.starts_with(char::is_whitespace) && line.contains("cl:lead"))
        .unwrap_or_else(|| panic!("the agent row must be rendered: {human}"));
    // The TABLE's third field is the declared state. An unobservable agent must
    // not have its liveness leak into that cell — the row still reports what the
    // agent DECLARED, and says nothing it cannot support about whether the pane
    // is there. That fact is what the digest's `alive` member is for.
    assert_eq!(
        agent_row.split_whitespace().nth(2),
        Some("-"),
        "an undeclared agent renders the no-declaration cell: {human}"
    );
    assert!(
        machine.contains(r#""alive":null"#),
        "and the machine surface says null: {machine}"
    );
    assert!(
        !machine.contains(r#""alive":false"#),
        "nothing was proven dead: {machine}"
    );
}
